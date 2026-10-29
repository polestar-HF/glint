//! Anthropic Messages API with SSE streaming.
//!
//! There is no official Rust SDK, so this speaks the HTTP API directly, as the API docs
//! describe it for raw HTTP:
//!
//! * `POST {base_url}/v1/messages` with `x-api-key` and `anthropic-version: 2026-10-21`.
//!   `base_url` defaults to `https://api.anthropic.com` and can point at a proxy; a base that
//!   already ends in `/v1` or `/v1/messages` is accepted.
//! * `stream: true`; text arrives as `content_block_delta` events with a `text_delta`.
//! * **No `temperature`, `top_p` or `top_k`.** On API version 2026-10-21 any value of these is
//!   a 400 on every model (and Haiku 5.5 already rejected non-default values before that), so
//!   determinism comes from the prompt and from low effort instead.
//! * Thinking is kept off or minimal for latency, and how depends on the model: see
//!   [`thinking_policy`].
//!
//! A server that ignores `stream: true` and answers with a plain JSON message is handled too,
//! because some proxies do that.

use serde_json::{json, Value};

use super::{non_empty, parse_err};
use crate::http::{
    is_event_stream, join_endpoint, retrying, send, stream_sse, Failure, Provider, Step, LLM_TIMEOUT,
};
use crate::prompt;
use crate::sse::SseEvent;
use crate::{DeltaSink, EngineConfig, Result, TranslateError, TranslateRequest, Translator};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2026-10-21";

pub(crate) struct Anthropic {
    id: String,
    name: String,
    api_key: Option<String>,
    model: String,
    url: String,
    extra_prompt: Option<String>,
    http: reqwest::Client,
}

impl Anthropic {
    pub fn new(id: String, name: String, cfg: &EngineConfig, http: reqwest::Client) -> Result<Self> {
        let api_key = non_empty(cfg.api_key.as_deref()).map(str::to_owned);
        let base = non_empty(cfg.base_url.as_deref());
        // A custom base URL may be a gateway that adds the key itself, so only the official
        // endpoint insists on one.
        if api_key.is_none() && base.is_none() {
            return Err(TranslateError::Config("The Claude API key is not set".into()));
        }
        Ok(Self {
            id,
            name,
            api_key,
            model: super::effective_model(cfg),
            url: join_endpoint(base.unwrap_or(DEFAULT_BASE_URL), "/v1/messages"),
            extra_prompt: non_empty(cfg.prompt.as_deref()).map(str::to_owned),
            http,
        })
    }

    async fn attempt(&self, body: &Value, deltas: &DeltaSink) -> std::result::Result<String, Failure> {
        let mut rb = self
            .http
            .post(&self.url)
            .timeout(LLM_TIMEOUT)
            .header("anthropic-version", API_VERSION)
            // Compressed streams may be buffered by intermediaries; first token matters here.
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .json(body);
        if let Some(key) = &self.api_key {
            rb = rb.header("x-api-key", key);
        }
        let resp = send(rb, Provider::Anthropic).await?;

        if !is_event_stream(&resp) {
            let text = resp.text().await?;
            let out = parse_message_json(&text)?;
            let _ = deltas.send(out.clone());
            return Ok(out);
        }

        let mut state = StreamState::default();
        let streamed = stream_sse(resp, deltas, |ev| state.on_event(ev)).await?;
        state.finish().map_err(|e| {
            // A stream that died early is worth one more try if nothing was shown yet.
            let f = if matches!(e, TranslateError::Network(_)) { Failure::retry(e) } else { Failure::fatal(e) };
            f.no_retry_if(streamed)
        })
    }
}

#[async_trait::async_trait]
impl Translator for Anthropic {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String> {
        if req.text.trim().is_empty() {
            return Ok(String::new());
        }
        let system = prompt::system_prompt(req, self.extra_prompt.as_deref());
        let user = prompt::user_message(&req.text);
        let body = request_body(&self.model, &system, &user, &req.text);
        retrying(|| self.attempt(&body, &deltas)).await
    }
}

/// How to keep thinking out of the way for a model.
///
/// Translation wants the first token fast and the output deterministic, so thinking is turned
/// off where the model allows it and kept minimal (effort `low`) where it does not. The rules
/// are per model family because they differ:
///
/// * Haiku 5.5, Opus 5, Sonnet 5: `thinking: {type: "disabled"}` is accepted at effort `high`
///   or below.
/// * Sonnet 5.5: `disabled` is a 400; `{type: "between_tools"}` is the way to turn it off.
/// * Opus 5.5 and the Fable/Mythos models: thinking cannot be turned off, so effort `low` it
///   is, and `max_tokens` gets headroom because thinking counts against it.
/// * Opus 4.6 to 4.8 and Sonnet 4.6: no thinking unless asked for, and they accept `effort`.
/// * Anything else (older Claude models, or a proxy's own model names): send neither field,
///   because `effort` is an error on Haiku 4.5 and Sonnet 4.5.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ThinkingPolicy {
    pub thinking: Option<Value>,
    pub effort: Option<&'static str>,
    /// Extra `max_tokens` to leave for thinking.
    pub headroom: u32,
}

pub(crate) fn thinking_policy(model: &str) -> ThinkingPolicy {
    let m = model.trim().to_ascii_lowercase();
    // Bedrock-style ids carry a vendor prefix.
    let m = m.strip_prefix("anthropic.").unwrap_or(&m);
    let off = || Some(json!({"type": "disabled"}));
    let low = Some("low");
    let (thinking, effort, headroom) = if m.starts_with("claude-haiku-5-5") {
        (off(), low, 0)
    } else if m.starts_with("claude-sonnet-5-5") {
        (Some(json!({"type": "between_tools"})), low, 0)
    } else if m.starts_with("claude-opus-5-5") {
        (None, low, 4096)
    } else if m.starts_with("claude-fable-") || m.starts_with("claude-mythos-") {
        (None, low, 4096)
    } else if m.starts_with("claude-opus-5") || m.starts_with("claude-sonnet-5") {
        (off(), low, 0)
    } else if ["claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"]
        .iter()
        .any(|p| m.starts_with(p))
    {
        (None, low, 0)
    } else {
        (None, None, 0)
    };
    ThinkingPolicy { thinking, effort, headroom }
}

/// The JSON body of the request.
pub(crate) fn request_body(model: &str, system: &str, user: &str, source_text: &str) -> Value {
    let policy = thinking_policy(model);
    let mut body = json!({
        "model": model,
        "max_tokens": prompt::max_output_tokens(source_text, policy.headroom),
        "stream": true,
        "system": system,
        "messages": [{"role": "user", "content": user}],
    });
    if let Some(t) = policy.thinking {
        body["thinking"] = t;
    }
    if let Some(e) = policy.effort {
        body["output_config"] = json!({"effort": e});
    }
    body
}

/// Accumulates a streamed message.
#[derive(Debug, Default)]
pub(crate) struct StreamState {
    text: String,
    stop_reason: Option<String>,
    stopped: bool,
}

impl StreamState {
    pub fn on_event(&mut self, ev: &SseEvent) -> std::result::Result<Step, TranslateError> {
        // `ping` and any event we do not know are ignored, as the API's versioning policy asks.
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            return Ok(Step::Continue);
        };
        let kind = v.get("type").and_then(Value::as_str).or(ev.event.as_deref());
        match kind {
            Some("content_block_delta") => {
                // `thinking_delta` and `signature_delta` are not part of the translation.
                if v.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") {
                    if let Some(t) = v.pointer("/delta/text").and_then(Value::as_str) {
                        self.text.push_str(t);
                        return Ok(Step::Emit(t.to_string()));
                    }
                }
                Ok(Step::Continue)
            }
            Some("message_delta") => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(r.to_string());
                }
                Ok(Step::Continue)
            }
            Some("message_stop") => {
                self.stopped = true;
                Ok(Step::Stop)
            }
            Some("error") => Err(api_error(&v)),
            _ => Ok(Step::Continue),
        }
    }

    pub fn finish(self) -> std::result::Result<String, TranslateError> {
        check_stop_reason(self.stop_reason.as_deref())?;
        if !self.stopped {
            return Err(TranslateError::Network(
                "The connection closed before the translation was complete".into(),
            ));
        }
        let text = self.text.trim().to_string();
        if text.is_empty() {
            return Err(TranslateError::Parse("the model returned no text".into()));
        }
        Ok(text)
    }
}

fn check_stop_reason(reason: Option<&str>) -> std::result::Result<(), TranslateError> {
    match reason {
        Some("refusal") => Err(TranslateError::Service {
            status: 200,
            message: "The model declined to translate this text".into(),
        }),
        Some("max_tokens") => {
            log::warn!("claude: the translation hit max_tokens and may be cut short");
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Map an API error object (`{"type":"error","error":{"type":...,"message":...}}`) to a
/// short error. The same shape appears as an HTTP error body and as a mid-stream event.
fn api_error(v: &Value) -> TranslateError {
    let status = match v.pointer("/error/type").and_then(Value::as_str).unwrap_or("") {
        "overloaded_error" => 529,
        "rate_limit_error" => 429,
        "authentication_error" => 401,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "invalid_request_error" => 400,
        _ => 500,
    };
    crate::http::service_error(status, &v.to_string(), Provider::Anthropic)
}

/// Parse a complete (non-streamed) message.
pub(crate) fn parse_message_json(body: &str) -> std::result::Result<String, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    if v.get("type").and_then(Value::as_str) == Some("error") {
        return Err(api_error(&v));
    }
    check_stop_reason(v.get("stop_reason").and_then(Value::as_str))?;
    let blocks = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("the response has no content"))?;
    let text: String = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(parse_err("the model returned no text"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(data: &str) -> SseEvent {
        SseEvent { event: None, data: data.to_string() }
    }

    /// Feed raw SSE text through the real parser and state machine.
    fn run_stream(raw: &str) -> (std::result::Result<String, TranslateError>, Vec<String>) {
        let mut parser = crate::sse::SseParser::new();
        let mut events = parser.feed(raw.as_bytes());
        events.extend(parser.finish());
        let mut state = StreamState::default();
        let mut deltas = Vec::new();
        for e in &events {
            match state.on_event(e) {
                Ok(Step::Emit(t)) => deltas.push(t),
                Ok(Step::Continue) => {}
                Ok(Step::Stop) => break,
                Err(err) => return (Err(err), deltas),
            }
        }
        (state.finish(), deltas)
    }

    const OK_STREAM: &str = "\
event: message_start
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-haiku-5-5\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":310,\"output_tokens\":1}}}

event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}

event: ping
data: {\"type\": \"ping\"}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好，\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"世界。\\n\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"第二行\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":14}}

event: message_stop
data: {\"type\":\"message_stop\"}

";

    #[test]
    fn streams_text_deltas_and_ignores_the_rest() {
        let (res, deltas) = run_stream(OK_STREAM);
        assert_eq!(deltas, vec!["你好，", "世界。\n", "第二行"]);
        assert_eq!(res.ok().as_deref(), Some("你好，世界。\n第二行"));
    }

    #[test]
    fn thinking_blocks_are_not_part_of_the_translation() {
        let raw = "\
event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"EqQB\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Bonjour\"}}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}

event: message_stop
data: {\"type\":\"message_stop\"}

";
        let (res, deltas) = run_stream(raw);
        assert_eq!(deltas, vec!["Bonjour"]);
        assert_eq!(res.ok().as_deref(), Some("Bonjour"));
    }

    #[test]
    fn mid_stream_error_event_maps_to_a_short_message() {
        let raw = "\
event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}

event: error
data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}

";
        let (res, deltas) = run_stream(raw);
        assert_eq!(deltas, vec!["Hel"]);
        match res {
            Err(TranslateError::Service { status: 529, message }) => assert_eq!(message, "The service is overloaded"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn error_types_map_to_statuses() {
        let e = |t: &str, m: &str| api_error(&json!({"type":"error","error":{"type":t,"message":m}}));
        assert!(matches!(e("rate_limit_error", "x"), TranslateError::Service { status: 429, ref message } if message == "Rate limited"));
        assert!(matches!(e("authentication_error", "x"), TranslateError::Service { status: 401, ref message } if message == "Invalid API key"));
        assert!(matches!(e("api_error", "x"), TranslateError::Service { status: 500, .. }));
        match e("invalid_request_error", "max_tokens: too large") {
            TranslateError::Service { status: 400, message } => assert_eq!(message, "Bad request: max_tokens: too large"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn refusal_is_reported_not_returned_as_text() {
        let raw = "\
event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\",\"stop_details\":{\"type\":\"refusal\",\"category\":\"cyber\",\"explanation\":null}}}

event: message_stop
data: {\"type\":\"message_stop\"}

";
        let (res, _) = run_stream(raw);
        match res {
            Err(TranslateError::Service { status: 200, message }) => assert!(message.contains("declined")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_stream_that_ends_early_is_a_network_error() {
        let raw = "\
event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}

";
        let (res, deltas) = run_stream(raw);
        assert_eq!(deltas, vec!["Hel"]);
        assert!(matches!(res, Err(TranslateError::Network(_))));
    }

    #[test]
    fn an_empty_answer_is_an_error() {
        let raw = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let (res, _) = run_stream(raw);
        assert!(matches!(res, Err(TranslateError::Parse(_))));
    }

    #[test]
    fn garbage_event_data_is_skipped() {
        let mut s = StreamState::default();
        assert!(matches!(s.on_event(&ev("not json")), Ok(Step::Continue)));
        assert!(matches!(s.on_event(&ev("{\"type\":\"something_new\"}")), Ok(Step::Continue)));
    }

    #[test]
    fn plain_json_message_is_parsed() {
        let body = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-haiku-5-5",
            "content":[{"type":"text","text":"Hello "},{"type":"text","text":"there"}],
            "stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":3}}"#;
        assert_eq!(parse_message_json(body).ok().as_deref(), Some("Hello there"));
        let with_thinking = r#"{"type":"message","content":[{"type":"thinking","thinking":"","signature":"x"},{"type":"text","text":"Hi"}],"stop_reason":"end_turn"}"#;
        assert_eq!(parse_message_json(with_thinking).ok().as_deref(), Some("Hi"));
        assert!(matches!(parse_message_json("<html>"), Err(TranslateError::Parse(_))));
        assert!(matches!(parse_message_json(r#"{"type":"message","content":[]}"#), Err(TranslateError::Parse(_))));
        assert!(matches!(
            parse_message_json(r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
            Err(TranslateError::Service { status: 401, .. })
        ));
        assert!(matches!(
            parse_message_json(r#"{"type":"message","content":[],"stop_reason":"refusal"}"#),
            Err(TranslateError::Service { status: 200, .. })
        ));
    }

    // ---- request construction --------------------------------------------------------------

    #[test]
    fn haiku_request_is_streaming_with_thinking_off_and_no_sampling_params() {
        let body = request_body("claude-haiku-5-5", "SYS", "<source_text>\nhi\n</source_text>", "hi");
        assert_eq!(body["model"], "claude-haiku-5-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "SYS");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "<source_text>\nhi\n</source_text>");
        assert_eq!(body["thinking"], json!({"type": "disabled"}));
        assert_eq!(body["output_config"], json!({"effort": "low"}));
        assert_eq!(body["max_tokens"], 256 + 2);
        for banned in ["temperature", "top_p", "top_k"] {
            assert!(body.get(banned).is_none(), "{banned} is a 400 on API version 2026-10-21");
        }
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1), "no assistant prefill");
    }

    #[test]
    fn thinking_policy_per_model_family() {
        let p = |m: &str| thinking_policy(m);
        let off = Some(json!({"type": "disabled"}));

        assert_eq!(p("claude-haiku-5-5"), ThinkingPolicy { thinking: off.clone(), effort: Some("low"), headroom: 0 });
        assert_eq!(p("anthropic.claude-haiku-5-5").thinking, off);
        assert_eq!(p("claude-sonnet-5-5").thinking, Some(json!({"type": "between_tools"})));
        assert_eq!(p("claude-sonnet-5-5").effort, Some("low"));
        // Opus 5.5 and Fable cannot turn thinking off: omit it, lowest effort, leave room.
        for m in ["claude-opus-5-5", "claude-fable-5-1", "claude-fable-5", "claude-mythos-5-1"] {
            let pol = p(m);
            assert_eq!(pol.thinking, None, "{m}");
            assert_eq!(pol.effort, Some("low"), "{m}");
            assert!(pol.headroom > 0, "{m}");
        }
        // Opus 5 and Sonnet 5 accept `disabled`; the 5-5 prefix must not catch them.
        assert_eq!(p("claude-opus-5").thinking, off);
        assert_eq!(p("claude-sonnet-5").thinking, off);
        // 4.6 to 4.8 do not think unless asked.
        for m in ["claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"] {
            assert_eq!(p(m), ThinkingPolicy { thinking: None, effort: Some("low"), headroom: 0 }, "{m}");
        }
        // Older models and unknown names get neither field (effort is an error on Haiku 4.5).
        for m in ["claude-haiku-4-5", "claude-sonnet-4-5", "claude-3-5-haiku-latest", "my-proxy-model", ""] {
            assert_eq!(p(m), ThinkingPolicy { thinking: None, effort: None, headroom: 0 }, "{m}");
        }
    }

    #[test]
    fn requests_for_other_models_follow_their_policy() {
        let b = request_body("claude-opus-5-5", "S", "U", "hi");
        assert!(b.get("thinking").is_none());
        assert_eq!(b["output_config"]["effort"], "low");
        assert_eq!(b["max_tokens"], 256 + 2 + 4096);
        let b = request_body("claude-sonnet-5-5", "S", "U", "hi");
        assert_eq!(b["thinking"], json!({"type": "between_tools"}));
        let b = request_body("claude-haiku-4-5", "S", "U", "hi");
        assert!(b.get("thinking").is_none() && b.get("output_config").is_none());
    }

    #[test]
    fn construction_requires_a_key_only_for_the_official_endpoint() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(2);
        assert!(matches!(
            Anthropic::new("c".into(), "C".into(), &cfg, http.clone()),
            Err(TranslateError::Config(_))
        ));
        cfg.api_key = Some("   ".into());
        assert!(Anthropic::new("c".into(), "C".into(), &cfg, http.clone()).is_err());
        cfg.api_key = Some(" sk-ant-x \n".into());
        let a = Anthropic::new("c".into(), "C".into(), &cfg, http.clone()).ok().expect("builds");
        assert_eq!(a.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(a.api_key.as_deref(), Some("sk-ant-x"));
        assert_eq!(a.model, "claude-haiku-5-5");

        cfg.api_key = None;
        cfg.base_url = Some("http://localhost:4000/v1/".into());
        let a = Anthropic::new("c".into(), "C".into(), &cfg, http).ok().expect("gateway needs no key");
        assert_eq!(a.url, "http://localhost:4000/v1/messages");
    }
}
