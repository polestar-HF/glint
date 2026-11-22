//! Microsoft / Bing Translator.
//!
//! Without an API key this uses Bing's web translator (`ttranslatev3`), since Microsoft retired
//! the old `edge.microsoft.com/translate/auth` endpoint (HTTP 404):
//!
//! 1. `GET https://www.bing.com/translator` (follows regional redirects such as
//!    `cn.bing.com/translator`) returns HTML containing `IG`, `data-iid="translator.xxxx"`, and
//!    `params_AbusePreventionHelper = [key, token, ttlMs]`. The session is cached in memory for
//!    its validity window (about 50 minutes) and refreshed automatically if rejected (`statusCode: 205`).
//! 2. `POST <origin>/ttranslatev3?isVertical=1&IG=<ig>&IID=<iid>` with form fields
//!    `fromLang`, `text`, `to`, `token`, `key` (HTTP/2 enabled on the shared client).
//!
//! That is unofficial and may change without notice ("free, best effort").
//!
//! With `api_key` set it uses the official Azure endpoint
//! (`https://api.cognitive.microsofttranslator.com`) with `Ocp-Apim-Subscription-Key`, and
//! `api_secret` as the resource region (`Ocp-Apim-Subscription-Region`; leave it empty for a
//! global resource).
//!
//! `base_url`, when set, replaces the host in both modes (in free mode, `{base_url}/translator`
//! and `{base_url}/ttranslatev3` are used, which lets tests and proxies stand in for Bing).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::header::{COOKIE, SET_COOKIE, USER_AGENT};
use serde_json::{json, Value};
use tokio::time::Instant;

use super::{non_empty, parse_err, Backend, MtOut};
use crate::http::{send, service_error, Failure, Provider, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{EngineConfig, Result, TranslateError};

const WEB_PAGE_URL: &str = "https://www.bing.com/translator";
const AZURE_API_BASE: &str = "https://api.cognitive.microsofttranslator.com";
const EDGE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";
/// Refresh a little before Bing's 1-hour expiry.
const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(50 * 60);
const MIN_SESSION_TTL: Duration = Duration::from_secs(60);
const MAX_SESSION_TTL: Duration = Duration::from_secs(55 * 60);

/// Process-wide cache for the default Bing web session so consecutive captures reuse the
/// ~50-minute token instead of fetching the 650 KB `/translator` HTML page every time.
fn shared_web_session() -> Arc<tokio::sync::Mutex<Option<WebSession>>> {
    static CELL: OnceLock<Arc<tokio::sync::Mutex<Option<WebSession>>>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(tokio::sync::Mutex::new(None)))
        .clone()
}

#[derive(Debug, Clone)]
struct WebSession {
    origin: String,
    ig: String,
    iid: String,
    key: String,
    token: String,
    cookie: Option<String>,
    fetched: Instant,
    ttl: Duration,
}

enum Mode {
    Web {
        page_url: String,
        session: Arc<tokio::sync::Mutex<Option<WebSession>>>,
    },
    Azure {
        base: String,
        key: String,
        region: Option<String>,
    },
}

pub(crate) struct Bing {
    mode: Mode,
    http: reqwest::Client,
}

impl Bing {
    pub fn new(cfg: &EngineConfig, http: reqwest::Client) -> Self {
        let custom_base =
            non_empty(cfg.base_url.as_deref()).map(|b| b.trim_end_matches('/').to_string());
        match non_empty(cfg.api_key.as_deref()) {
            Some(key) => Bing {
                mode: Mode::Azure {
                    base: custom_base.unwrap_or_else(|| AZURE_API_BASE.to_string()),
                    key: key.to_string(),
                    region: non_empty(cfg.api_secret.as_deref()).map(str::to_owned),
                },
                http,
            },
            None => {
                // Custom bases (proxies / mock servers) get an isolated session cache so tests
                // never share state across mock instances.
                let (page_url, session) = match custom_base {
                    Some(b) => (
                        format!("{b}/translator"),
                        Arc::new(tokio::sync::Mutex::new(None)),
                    ),
                    None => (WEB_PAGE_URL.to_string(), shared_web_session()),
                };
                Bing {
                    mode: Mode::Web { page_url, session },
                    http,
                }
            }
        }
    }

    async fn web_session(
        &self,
        page_url: &str,
        cache: &tokio::sync::Mutex<Option<WebSession>>,
    ) -> std::result::Result<WebSession, Failure> {
        let mut slot = cache.lock().await;
        if let Some(s) = slot.as_ref() {
            if s.fetched.elapsed() < s.ttl {
                return Ok(s.clone());
            }
        }
        let rb = self
            .http
            .get(page_url)
            .timeout(MT_TIMEOUT)
            .header(USER_AGENT, EDGE_USER_AGENT);
        let resp = send(rb, Provider::Bing).await?;
        let final_url = resp.url().clone();
        let origin = url_origin(&final_url);
        let cookie = collect_cookies(resp.headers());
        let html = resp.text().await?;
        let parsed = parse_web_session(&html)?;
        let sess = WebSession {
            origin,
            ig: parsed.ig,
            iid: parsed.iid,
            key: parsed.key,
            token: parsed.token,
            cookie,
            fetched: Instant::now(),
            ttl: parsed.ttl,
        };
        *slot = Some(sess.clone());
        Ok(sess)
    }

    async fn call_web(
        &self,
        page_url: &str,
        cache: &tokio::sync::Mutex<Option<WebSession>>,
        text: &str,
        from: &str,
        to: &str,
    ) -> std::result::Result<MtOut, Failure> {
        let sess = self.web_session(page_url, cache).await?;
        let from_lang = if from.is_empty() { "auto-detect" } else { from };
        let mut rb = self
            .http
            .post(format!("{}/ttranslatev3", sess.origin))
            .timeout(MT_TIMEOUT)
            .header(USER_AGENT, EDGE_USER_AGENT)
            .query(&[
                ("isVertical", "1"),
                ("IG", sess.ig.as_str()),
                ("IID", sess.iid.as_str()),
            ])
            .form(&[
                ("fromLang", from_lang),
                ("text", text),
                ("to", to),
                ("token", sess.token.as_str()),
                ("key", sess.key.as_str()),
            ]);
        if let Some(cookie) = &sess.cookie {
            rb = rb.header(COOKIE, cookie);
        }

        let body = match send(rb, Provider::Bing).await {
            Ok(resp) => resp.text().await?,
            Err(f) => {
                if matches!(
                    &f.err,
                    TranslateError::Service {
                        status: 401 | 403,
                        ..
                    }
                ) {
                    *cache.lock().await = None;
                    return Err(Failure::retry(f.err));
                }
                return Err(f);
            }
        };

        // Bing's web endpoint reports token expiry / errors as HTTP 200 with `{"statusCode": ...}`.
        if let Some(code) = embedded_status_code(&body) {
            return match code {
                205 | 206 | 401 => {
                    *cache.lock().await = None;
                    Err(Failure::retry(TranslateError::Service {
                        status: 401,
                        message: "Authentication failed".into(),
                    }))
                }
                429 => Err(Failure::fatal(TranslateError::RateLimited {
                    message: "Rate limited".into(),
                    retry_after: None,
                })),
                other => Err(Failure::from(service_error(other, &body, Provider::Bing))),
            };
        }

        Ok(parse_response(&body)?)
    }

    async fn call_azure(
        &self,
        base: &str,
        key: &str,
        region: Option<&str>,
        text: &str,
        from: &str,
        to: &str,
    ) -> std::result::Result<MtOut, Failure> {
        let mut query = vec![("api-version", "3.0"), ("to", to), ("textType", "plain")];
        if !from.is_empty() {
            query.push(("from", from));
        }
        let mut rb = self
            .http
            .post(format!("{base}/translate"))
            .timeout(MT_TIMEOUT)
            .query(&query)
            .header("Ocp-Apim-Subscription-Key", key)
            .json(&json!([{ "Text": text }]));
        if let Some(r) = region {
            rb = rb.header("Ocp-Apim-Subscription-Region", r);
        }
        let resp = send(rb, Provider::Bing).await?;
        Ok(parse_response(&resp.text().await?)?)
    }
}

#[async_trait::async_trait]
impl Backend for Bing {
    fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
        langs::bing(code, role)
    }

    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        match &self.mode {
            Mode::Web { page_url, session } => {
                self.call_web(page_url, session, text, from, to).await
            }
            Mode::Azure { base, key, region } => {
                self.call_azure(base, key, region.as_deref(), text, from, to)
                    .await
            }
        }
    }
}

fn url_origin(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("www.bing.com");
    match url.port() {
        Some(p) => format!("{}://{host}:{p}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn collect_cookies(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let parts: Vec<&str> = headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .map(str::trim)
        .filter(|s| s.contains('='))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedWebSession {
    ig: String,
    iid: String,
    key: String,
    token: String,
    ttl: Duration,
}

/// Extract `IG`, `data-iid`, and `params_AbusePreventionHelper = [key, "token", ttlMs]`
/// from the Bing Translator HTML page.
fn parse_web_session(html: &str) -> std::result::Result<ParsedWebSession, TranslateError> {
    let (key, token, ttl_ms) = extract_abuse_helper(html)
        .ok_or_else(|| parse_err("could not find Bing translator token in page"))?;
    let ig = extract_ig(html).ok_or_else(|| parse_err("could not find Bing IG in page"))?;
    let iid = extract_iid(html).ok_or_else(|| parse_err("could not find Bing IID in page"))?;
    let ttl = if ttl_ms > 0 {
        Duration::from_millis(ttl_ms)
            .saturating_sub(Duration::from_secs(5 * 60))
            .clamp(MIN_SESSION_TTL, MAX_SESSION_TTL)
    } else {
        DEFAULT_SESSION_TTL
    };
    Ok(ParsedWebSession {
        ig,
        iid,
        key,
        token,
        ttl,
    })
}

fn extract_abuse_helper(html: &str) -> Option<(String, String, u64)> {
    let idx = html.find("params_AbusePreventionHelper")?;
    let after = &html[idx + "params_AbusePreventionHelper".len()..];
    let bracket = after.find('[')?;
    let rest = &after[bracket + 1..];
    let close = rest.find(']')?;
    let inner = &rest[..close];

    // Expect: `<key_digits>, "<token>", <ttl_ms>`
    let mut parts = inner.splitn(3, ',');
    let key = parts.next()?.trim();
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let raw_token = parts.next()?.trim();
    let token = raw_token.strip_prefix('"')?.strip_suffix('"')?;
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return None;
    }
    let ttl_ms = parts
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Some((key.to_string(), token.to_string(), ttl_ms))
}

fn extract_ig(html: &str) -> Option<String> {
    for prefix in [r#"IG:""#, r#"IG=""#] {
        let mut cursor = html;
        while let Some(pos) = cursor.find(prefix) {
            let rest = &cursor[pos + prefix.len()..];
            if let Some(end) = rest.find('"') {
                let candidate = &rest[..end];
                if (16..=64).contains(&candidate.len())
                    && candidate.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Some(candidate.to_string());
                }
            }
            cursor = rest;
        }
    }
    None
}

fn extract_iid(html: &str) -> Option<String> {
    // Prefer the translator widget's own `data-iid="translator.xxxx"` over unrelated page elements.
    for prefix in [r#"data-iid="translator."#, r#"data-iid=""#] {
        if let Some(pos) = html.find(prefix) {
            let value_start = pos + r#"data-iid=""#.len();
            let rest = &html[value_start..];
            if let Some(end) = rest.find('"') {
                let candidate = rest[..end].trim();
                if !candidate.is_empty() && candidate.len() <= 64 {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

/// Detect `{"statusCode": 205}`-style error envelopes that Bing returns with HTTP 200.
fn embedded_status_code(body: &str) -> Option<u16> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    let code = v.get("statusCode")?.as_u64()?;
    u16::try_from(code).ok()
}

/// Parse `[{"detectedLanguage":{"language":"en","score":1.0},"translations":[{"text":"..","to":".."}]}]`.
pub(crate) fn parse_response(body: &str) -> std::result::Result<MtOut, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    let first = v
        .get(0)
        .ok_or_else(|| parse_err("the response has no translation"))?;
    let text = first
        .pointer("/translations/0/text")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_err("the response has no translated text"))?;
    Ok(MtOut {
        text: text.to_string(),
        detected: first
            .pointer("/detectedLanguage/language")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_translation_with_detection() {
        let body = r#"[{"detectedLanguage":{"language":"en","score":1.0},"translations":[{"text":"你好，世界。\n第二行","to":"zh-Hans"}]}]"#;
        let out = parse_response(body).expect("parses");
        assert_eq!(out.text, "你好，世界。\n第二行");
        assert_eq!(out.detected.as_deref(), Some("en"));
    }

    #[test]
    fn detection_is_optional_when_the_source_was_given() {
        let body = r#"[{"translations":[{"text":"Hola","to":"es"}]}]"#;
        let out = parse_response(body).expect("parses");
        assert_eq!((out.text.as_str(), out.detected), ("Hola", None));
    }

    #[test]
    fn unexpected_bodies_are_parse_errors() {
        for body in [
            "",
            "[]",
            "{}",
            r#"[{}]"#,
            r#"[{"translations":[]}]"#,
            r#"{"error":{"code":401000,"message":"x"}}"#,
            "<html>",
        ] {
            assert!(
                matches!(parse_response(body), Err(TranslateError::Parse(_))),
                "{body:?}"
            );
        }
    }

    #[test]
    fn parses_web_session_from_translator_html() {
        let html = r#"
            <html><head><script>
            _G={IG:"0123456789ABCDEF0123456789ABCDEF"};
            var params_AbusePreventionHelper = [1788995349630,"3YwdKK5ea9-7_tok",3600000];
            </script></head><body>
            <div data-iid="Header.1"></div>
            <div id="rich_tta" data-iid="translator.5023"></div>
            </body></html>
        "#;
        let s = parse_web_session(html).expect("parses");
        assert_eq!(s.ig, "0123456789ABCDEF0123456789ABCDEF");
        assert_eq!(s.iid, "translator.5023");
        assert_eq!(s.key, "1788995349630");
        assert_eq!(s.token, "3YwdKK5ea9-7_tok");
        assert_eq!(s.ttl, Duration::from_secs(55 * 60));

        assert!(parse_web_session("<html>nothing here</html>").is_err());
    }

    #[test]
    fn detects_embedded_status_codes() {
        assert_eq!(embedded_status_code(r#"{"statusCode":205}"#), Some(205));
        assert_eq!(embedded_status_code(r#"{"statusCode":429}"#), Some(429));
        assert_eq!(
            embedded_status_code(r#"[{"translations":[{"text":"ok"}]}]"#),
            None
        );
    }

    #[test]
    fn mode_and_hosts_follow_the_config() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(1);

        let b = Bing::new(&cfg, http.clone());
        assert!(
            matches!(&b.mode, Mode::Web { page_url, .. } if page_url == "https://www.bing.com/translator")
        );

        cfg.base_url = Some("http://127.0.0.1:9/".into());
        let b = Bing::new(&cfg, http.clone());
        assert!(
            matches!(&b.mode, Mode::Web { page_url, .. } if page_url == "http://127.0.0.1:9/translator")
        );

        cfg.base_url = None;
        cfg.api_key = Some("azure-key".into());
        cfg.api_secret = Some(" westeurope ".into());
        let b = Bing::new(&cfg, http.clone());
        assert!(matches!(&b.mode, Mode::Azure { base, key, region }
                if base == "https://api.cognitive.microsofttranslator.com"
                && key == "azure-key"
                && region.as_deref() == Some("westeurope")));

        cfg.api_secret = Some("".into());
        let b = Bing::new(&cfg, http);
        assert!(matches!(&b.mode, Mode::Azure { region: None, .. }));
    }
}
