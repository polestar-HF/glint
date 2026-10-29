//! Shared HTTP plumbing: client construction, error mapping, the single retry, and the loop
//! that reads a streaming response.

use std::future::Future;
use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use serde_json::Value;

use crate::sse::{SseEvent, SseParser};
use crate::{DeltaSink, TranslateError};

/// Connect timeout. It can only be set on the client, so [`http_client`] sets it.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Total request timeout for machine-translation engines.
pub(crate) const MT_TIMEOUT: Duration = Duration::from_secs(30);
/// Total request timeout for LLM engines (includes the time the stream stays open).
pub(crate) const LLM_TIMEOUT: Duration = Duration::from_secs(60);
/// Pause before the single retry, unless the server said how long to wait.
const RETRY_DELAY: Duration = Duration::from_millis(400);
/// The longest `Retry-After` we are willing to sit through.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3);
/// How much of an error body is read. The body is only mined for a short message.
const MAX_ERROR_BODY: usize = 64 * 1024;

/// A client configured the way the engines expect: rustls, a 5 s connect timeout and a
/// `User-Agent`. Engines apply their own total timeout per request, so any `reqwest::Client`
/// works, but only this one enforces the connect timeout.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("Glint/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Which service an error came from; a few statuses mean different things per service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provider {
    Anthropic,
    OpenAi,
    Google,
    Bing,
    DeepL,
    Generic,
}

/// An error plus what the retry logic needs to know about it.
#[derive(Debug)]
pub(crate) struct Failure {
    pub err: TranslateError,
    pub retryable: bool,
    pub retry_after: Option<Duration>,
}

impl Failure {
    pub fn fatal(err: TranslateError) -> Self {
        Failure {
            err,
            retryable: false,
            retry_after: None,
        }
    }

    pub fn retry(err: TranslateError) -> Self {
        Failure {
            err,
            retryable: true,
            retry_after: None,
        }
    }

    /// A retry is only allowed while nothing has been streamed to the caller.
    pub fn no_retry_if(mut self, streamed: bool) -> Self {
        if streamed {
            self.retryable = false;
        }
        self
    }
}

impl From<TranslateError> for Failure {
    fn from(err: TranslateError) -> Self {
        let retryable =
            matches!(&err, TranslateError::Service { status, .. } if is_retryable_status(*status));
        Failure {
            err,
            retryable,
            retry_after: None,
        }
    }
}

impl From<reqwest::Error> for Failure {
    fn from(e: reqwest::Error) -> Self {
        let (message, retryable) = if e.is_timeout() && !e.is_connect() {
            // The request ran out of its total time budget: another attempt would only
            // double the wait.
            ("The request timed out".to_string(), false)
        } else if e.is_connect() {
            ("Could not connect to the server".to_string(), true)
        } else if e.is_body() || e.is_decode() || e.is_request() {
            ("The connection was lost".to_string(), true)
        } else {
            (format!("Request failed: {}", e.without_url()), false)
        };
        Failure {
            err: TranslateError::Network(message),
            retryable,
            retry_after: None,
        }
    }
}

pub(crate) fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// Run `op`, and run it once more if it failed in a retryable way.
pub(crate) async fn retrying<T, F, Fut>(mut op: F) -> Result<T, TranslateError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Failure>>,
{
    match op().await {
        Ok(v) => Ok(v),
        Err(f) if f.retryable => {
            log::warn!("translate request failed ({}), retrying once", f.err);
            tokio::time::sleep(f.retry_after.unwrap_or(RETRY_DELAY).min(MAX_RETRY_AFTER)).await;
            op().await.map_err(|f| f.err)
        }
        Err(f) => Err(f.err),
    }
}

/// Send a request and turn every non-2xx answer into a short, human-readable [`Failure`].
pub(crate) async fn send(
    req: reqwest::RequestBuilder,
    provider: Provider,
) -> Result<reqwest::Response, Failure> {
    let resp = req.send().await?;
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = read_limited(resp).await;
    let mut failure = Failure::from(service_error(status, &body, provider));
    failure.retry_after = retry_after;
    Err(failure)
}

async fn read_limited(mut resp: reqwest::Response) -> String {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < MAX_ERROR_BODY {
        match resp.chunk().await {
            Ok(Some(c)) => buf.extend_from_slice(&c),
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Map an HTTP status (and, for a few statuses, the body) to a short message.
pub(crate) fn service_error(status: u16, body: &str, provider: Provider) -> TranslateError {
    let lower = body.to_ascii_lowercase();
    let base: &str = match status {
        400 => "Bad request",
        401 => match provider {
            Provider::Bing => "Authentication failed",
            _ => "Invalid API key",
        },
        402 => "Quota exceeded",
        403 => match provider {
            Provider::DeepL => "Invalid API key",
            _ => "Access denied",
        },
        404 => "Not found (check the base URL and model name)",
        408 => "The request timed out",
        413 => "The text is too long",
        422 => "The request was rejected",
        429 => {
            if lower.contains("insufficient_quota") {
                "Quota exceeded"
            } else {
                "Rate limited"
            }
        }
        456 => "Quota exceeded",
        529 => "The service is overloaded",
        500..=599 => {
            if matches!(status, 502..=504) {
                "The service is unavailable"
            } else {
                "Server error"
            }
        }
        _ => "Request failed",
    };
    // Only these statuses carry a detail that helps the user fix something.
    let message = if matches!(status, 400 | 403 | 404 | 422) {
        match error_detail(body) {
            Some(d) if provider != Provider::DeepL || status != 403 => format!("{base}: {d}"),
            _ => base.to_string(),
        }
    } else {
        base.to_string()
    };
    TranslateError::Service { status, message }
}

/// Pull a short message out of the usual error body shapes, or `None`.
pub(crate) fn error_detail(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let from_json = serde_json::from_str::<Value>(trimmed).ok().and_then(|v| {
        [
            v.pointer("/error/message"),
            v.pointer("/error/error/message"),
            v.get("error"),
            v.get("message"),
            v.get("detail"),
            v.get("error_msg"),
        ]
        .into_iter()
        .flatten()
        .find_map(|x| x.as_str().map(str::to_owned))
    });
    let text = match from_json {
        Some(t) => t,
        // Plain text is only worth showing if it is short and not an HTML error page.
        None if !trimmed.starts_with('{') && !trimmed.contains('<') && trimmed.len() <= 160 => {
            trimmed.to_string()
        }
        None => return None,
    };
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return None;
    }
    Some(truncate_chars(&one_line, 200))
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// What a stream handler wants done after seeing an event.
pub(crate) enum Step {
    /// Nothing to forward.
    Continue,
    /// Forward this text to the caller.
    Emit(String),
    /// The stream is complete; stop reading.
    Stop,
}

/// Read an event stream to its end, forwarding text to `deltas`.
///
/// `handle` sees every event. Returns `true` when at least one delta was sent. An error after
/// the first delta is marked non-retryable, because the caller already has partial output.
pub(crate) async fn stream_sse(
    mut resp: reqwest::Response,
    deltas: &DeltaSink,
    mut handle: impl FnMut(&SseEvent) -> Result<Step, TranslateError>,
) -> Result<bool, Failure> {
    let mut parser = SseParser::new();
    let mut emitted = false;
    let mut stopped = false;
    while !stopped {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => return Err(Failure::from(e).no_retry_if(emitted)),
        };
        for ev in parser.feed(&chunk) {
            match handle(&ev).map_err(|e| Failure::from(e).no_retry_if(emitted))? {
                Step::Continue => {}
                Step::Emit(text) => {
                    if !text.is_empty() {
                        emitted = true;
                        // The receiver going away just means nobody is listening any more.
                        let _ = deltas.send(text);
                    }
                }
                Step::Stop => {
                    stopped = true;
                    break;
                }
            }
        }
    }
    if !stopped {
        for ev in parser.finish() {
            match handle(&ev).map_err(|e| Failure::from(e).no_retry_if(emitted))? {
                Step::Continue => {}
                Step::Emit(text) => {
                    if !text.is_empty() {
                        emitted = true;
                        let _ = deltas.send(text);
                    }
                }
                Step::Stop => break,
            }
        }
    }
    Ok(emitted)
}

/// Whether a response is an event stream (as opposed to a plain JSON body from a server that
/// ignored `stream: true`).
pub(crate) fn is_event_stream(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"))
}

/// Join a base URL and an API path, tolerating a trailing slash and a base that already ends
/// with the version segment or the full endpoint.
///
/// `join_endpoint("https://h/v1/", "/v1/messages")` and `join_endpoint("https://h", "/v1/messages")`
/// both give a single `/v1/messages`.
pub(crate) fn join_endpoint(base: &str, endpoint: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let endpoint = endpoint.trim_start_matches('/');
    if base.ends_with(endpoint) {
        return base.to_string();
    }
    // Endpoint like "v1/messages" and a base ending in "/v1": do not repeat "v1".
    if let Some((first, rest)) = endpoint.split_once('/') {
        if first.len() <= 3 && first.starts_with('v') && base.ends_with(&format!("/{first}")) {
            return format!("{base}/{rest}");
        }
    }
    format!("{base}/{endpoint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(status: u16, body: &str, p: Provider) -> String {
        match service_error(status, body, p) {
            TranslateError::Service { status: s, message } => {
                assert_eq!(s, status);
                message
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn statuses_map_to_short_messages() {
        assert_eq!(
            msg(
                401,
                "{\"error\":{\"message\":\"bad key sk-123\"}}",
                Provider::OpenAi
            ),
            "Invalid API key"
        );
        assert_eq!(msg(402, "", Provider::Generic), "Quota exceeded");
        assert_eq!(msg(429, "", Provider::Anthropic), "Rate limited");
        assert_eq!(msg(456, "", Provider::DeepL), "Quota exceeded");
        assert_eq!(
            msg(403, "{\"message\":\"Forbidden\"}", Provider::DeepL),
            "Invalid API key"
        );
        assert_eq!(msg(401, "", Provider::Bing), "Authentication failed");
        assert_eq!(
            msg(500, "<html>boom</html>", Provider::Generic),
            "Server error"
        );
        assert_eq!(
            msg(503, "", Provider::Generic),
            "The service is unavailable"
        );
        assert_eq!(
            msg(529, "", Provider::Anthropic),
            "The service is overloaded"
        );
        assert_eq!(msg(418, "", Provider::Generic), "Request failed");
    }

    #[test]
    fn openai_quota_is_not_reported_as_rate_limit() {
        let body = r#"{"error":{"message":"You exceeded your current quota","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        assert_eq!(msg(429, body, Provider::OpenAi), "Quota exceeded");
        let body = r#"{"error":{"message":"Rate limit reached for requests","type":"requests","code":"rate_limit_exceeded"}}"#;
        assert_eq!(msg(429, body, Provider::OpenAi), "Rate limited");
    }

    #[test]
    fn bad_request_includes_the_service_detail_but_never_a_body_dump() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"model: claude-nope is not a valid model"}}"#;
        assert_eq!(
            msg(400, body, Provider::Anthropic),
            "Bad request: model: claude-nope is not a valid model"
        );
        assert_eq!(
            msg(404, "{\"error\":\"model 'x' not found\"}", Provider::OpenAi),
            "Not found (check the base URL and model name): model 'x' not found"
        );
        // An HTML error page is not shown.
        assert_eq!(
            msg(400, "<html><body>Bad</body></html>", Provider::Generic),
            "Bad request"
        );
        // A very long detail is cut.
        let long = format!("{{\"message\":\"{}\"}}", "x".repeat(500));
        let m = msg(400, &long, Provider::Generic);
        assert!(m.chars().count() < 230, "{m}");
        assert!(m.ends_with('…'));
    }

    #[test]
    fn error_detail_shapes() {
        assert_eq!(
            error_detail(r#"{"error":{"message":"a  b\nc"}}"#).as_deref(),
            Some("a b c")
        );
        assert_eq!(
            error_detail(r#"{"error":"plain"}"#).as_deref(),
            Some("plain")
        );
        assert_eq!(error_detail(r#"{"message":"m"}"#).as_deref(), Some("m"));
        assert_eq!(error_detail(r#"{"detail":"d"}"#).as_deref(), Some("d"));
        assert_eq!(
            error_detail(r#"{"error_msg":"UNAUTHORIZED USER"}"#).as_deref(),
            Some("UNAUTHORIZED USER")
        );
        assert_eq!(error_detail("Not Found").as_deref(), Some("Not Found"));
        assert_eq!(error_detail(""), None);
        assert_eq!(error_detail("{\"weird\":1}"), None);
    }

    #[test]
    fn retryable_statuses() {
        for s in [429, 500, 502, 503, 504, 529] {
            assert!(
                Failure::from(TranslateError::Service {
                    status: s,
                    message: String::new()
                })
                .retryable,
                "{s}"
            );
        }
        for s in [400, 401, 402, 403, 404, 413, 456] {
            assert!(
                !Failure::from(TranslateError::Service {
                    status: s,
                    message: String::new()
                })
                .retryable,
                "{s}"
            );
        }
        assert!(!Failure::from(TranslateError::Parse("x".into())).retryable);
        assert!(!Failure::from(TranslateError::Config("x".into())).retryable);
    }

    #[test]
    fn no_retry_after_streaming() {
        let f = Failure::retry(TranslateError::Network("x".into()));
        assert!(f.retryable);
        assert!(!f.no_retry_if(true).retryable);
        assert!(
            Failure::retry(TranslateError::Network("x".into()))
                .no_retry_if(false)
                .retryable
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retrying_runs_at_most_twice() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = AtomicU32::new(0);
        let out: Result<(), TranslateError> = retrying(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            async {
                Err(Failure::from(TranslateError::Service {
                    status: 503,
                    message: "x".into(),
                }))
            }
        })
        .await;
        assert!(out.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        let calls = AtomicU32::new(0);
        let out: Result<u32, TranslateError> = retrying(|| {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Err(Failure::retry(TranslateError::Network("blip".into())))
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(out.ok(), Some(7));

        let calls = AtomicU32::new(0);
        let out: Result<(), TranslateError> = retrying(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            async {
                Err(Failure::from(TranslateError::Service {
                    status: 401,
                    message: "x".into(),
                }))
            }
        })
        .await;
        assert!(out.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1, "401 is never retried");
    }

    #[test]
    fn join_endpoint_variants() {
        for base in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "https://api.anthropic.com/v1",
            "https://api.anthropic.com/v1/",
            "https://api.anthropic.com/v1/messages",
        ] {
            assert_eq!(
                join_endpoint(base, "/v1/messages"),
                "https://api.anthropic.com/v1/messages",
                "{base}"
            );
        }
        assert_eq!(
            join_endpoint("https://h/openai/v1", "/chat/completions"),
            "https://h/openai/v1/chat/completions"
        );
        assert_eq!(
            join_endpoint("https://h/v1/chat/completions", "/chat/completions"),
            "https://h/v1/chat/completions"
        );
        assert_eq!(
            join_endpoint(" http://127.0.0.1:11434/v1/ ", "chat/completions"),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }
}
