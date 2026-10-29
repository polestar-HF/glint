//! Microsoft Translator.
//!
//! Without an API key this uses the endpoint that Microsoft Edge's built-in translator uses:
//!
//! 1. `GET https://edge.microsoft.com/translate/auth` returns a short-lived JWT as plain text.
//!    The token is cached for 8 minutes and shared by all requests of this engine, and it is
//!    dropped and fetched again if the translate endpoint rejects it.
//! 2. `POST https://api-edge.cognitive.microsofttranslator.com/translate?api-version=3.0` with
//!    `Authorization: Bearer <jwt>` and a JSON body `[{"Text": ...}]`.
//!
//! That is unofficial and may change without notice ("free, best effort").
//!
//! With `api_key` set it uses the official Azure endpoint
//! (`https://api.cognitive.microsofttranslator.com`) with `Ocp-Apim-Subscription-Key`, and
//! `api_secret` as the resource region (`Ocp-Apim-Subscription-Region`; leave it empty for a
//! global resource).
//!
//! `base_url`, when set, replaces the translate host. In free mode the token is then fetched
//! from `{base_url}/translate/auth` as well, which is what lets tests and proxies stand in for
//! both hosts.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::Instant;

use super::{non_empty, parse_err, Backend, MtOut};
use crate::http::{send, Failure, Provider, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{EngineConfig, Result, TranslateError};

const EDGE_AUTH_URL: &str = "https://edge.microsoft.com/translate/auth";
const EDGE_API_BASE: &str = "https://api-edge.cognitive.microsofttranslator.com";
const AZURE_API_BASE: &str = "https://api.cognitive.microsofttranslator.com";
/// The Edge token is valid for about 10 minutes; refresh a little early.
const TOKEN_TTL: Duration = Duration::from_secs(8 * 60);

enum Mode {
    Edge {
        auth_url: String,
        token: tokio::sync::Mutex<Option<Token>>,
    },
    Azure {
        key: String,
        region: Option<String>,
    },
}

struct Token {
    value: String,
    fetched: Instant,
}

pub(crate) struct Bing {
    mode: Mode,
    base: String,
    http: reqwest::Client,
}

impl Bing {
    pub fn new(cfg: &EngineConfig, http: reqwest::Client) -> Self {
        let custom_base =
            non_empty(cfg.base_url.as_deref()).map(|b| b.trim_end_matches('/').to_string());
        match non_empty(cfg.api_key.as_deref()) {
            Some(key) => Bing {
                mode: Mode::Azure {
                    key: key.to_string(),
                    region: non_empty(cfg.api_secret.as_deref()).map(str::to_owned),
                },
                base: custom_base.unwrap_or_else(|| AZURE_API_BASE.to_string()),
                http,
            },
            None => {
                let auth_url = match &custom_base {
                    Some(b) => format!("{b}/translate/auth"),
                    None => EDGE_AUTH_URL.to_string(),
                };
                Bing {
                    mode: Mode::Edge {
                        auth_url,
                        token: tokio::sync::Mutex::new(None),
                    },
                    base: custom_base.unwrap_or_else(|| EDGE_API_BASE.to_string()),
                    http,
                }
            }
        }
    }

    /// A valid Edge token, fetched if there is none or it is older than the TTL.
    async fn edge_token(
        &self,
        auth_url: &str,
        cache: &tokio::sync::Mutex<Option<Token>>,
    ) -> std::result::Result<String, Failure> {
        // Holding the lock across the fetch means concurrent requests wait for one fetch
        // instead of each making their own.
        let mut slot = cache.lock().await;
        if let Some(t) = slot.as_ref() {
            if t.fetched.elapsed() < TOKEN_TTL {
                return Ok(t.value.clone());
            }
        }
        let rb = self.http.get(auth_url).timeout(MT_TIMEOUT);
        let body = send(rb, Provider::Bing).await?.text().await?;
        let value = validate_token(&body)?;
        *slot = Some(Token {
            value: value.clone(),
            fetched: Instant::now(),
        });
        Ok(value)
    }
}

#[async_trait::async_trait]
impl Backend for Bing {
    fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
        langs::bing(code, role)
    }

    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        let mut query = vec![("api-version", "3.0"), ("to", to), ("textType", "plain")];
        if !from.is_empty() {
            query.push(("from", from));
        }
        let mut rb = self
            .http
            .post(format!("{}/translate", self.base))
            .timeout(MT_TIMEOUT)
            .query(&query)
            .json(&json!([{ "Text": text }]));

        match &self.mode {
            Mode::Edge { auth_url, token } => {
                rb = rb.bearer_auth(self.edge_token(auth_url, token).await?);
            }
            Mode::Azure { key, region } => {
                rb = rb.header("Ocp-Apim-Subscription-Key", key);
                if let Some(r) = region {
                    rb = rb.header("Ocp-Apim-Subscription-Region", r);
                }
            }
        }

        match send(rb, Provider::Bing).await {
            Ok(resp) => Ok(parse_response(&resp.text().await?)?),
            Err(f) => {
                if let (Mode::Edge { token, .. }, TranslateError::Service { status: 401, .. }) =
                    (&self.mode, &f.err)
                {
                    // The token expired early or was revoked: forget it, and let the retry
                    // fetch a new one.
                    *token.lock().await = None;
                    return Err(Failure::retry(f.err));
                }
                Err(f)
            }
        }
    }
}

/// The auth endpoint answers with the bare JWT. Reject anything else, such as an HTML page
/// from a captive portal, rather than sending it as a credential.
pub(crate) fn validate_token(body: &str) -> std::result::Result<String, TranslateError> {
    let t = body.trim();
    if t.len() < 20 || t.len() > 8192 || t.chars().any(char::is_whitespace) || t.contains('<') {
        return Err(parse_err("the Bing token response is not a token"));
    }
    Ok(t.to_string())
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
    fn token_validation() {
        let jwt = "eyJhbGciOiJFUzI1NiIsImtpZCI6ImtleTEiLCJ0eXAiOiJKV1QifQ.eyJzdWIiOiJ4In0.c2ln";
        assert_eq!(
            validate_token(&format!("{jwt}\n")).ok().as_deref(),
            Some(jwt)
        );
        for bad in [
            "",
            "short",
            "<html><body>Sign in to the network please</body></html>",
            "two words here are not a token ever",
        ] {
            assert!(validate_token(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn mode_and_hosts_follow_the_config() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(1);

        let b = Bing::new(&cfg, http.clone());
        assert_eq!(b.base, "https://api-edge.cognitive.microsofttranslator.com");
        assert!(
            matches!(&b.mode, Mode::Edge { auth_url, .. } if auth_url == "https://edge.microsoft.com/translate/auth")
        );

        cfg.base_url = Some("http://127.0.0.1:9/".into());
        let b = Bing::new(&cfg, http.clone());
        assert_eq!(b.base, "http://127.0.0.1:9");
        assert!(
            matches!(&b.mode, Mode::Edge { auth_url, .. } if auth_url == "http://127.0.0.1:9/translate/auth")
        );

        cfg.base_url = None;
        cfg.api_key = Some("azure-key".into());
        cfg.api_secret = Some(" westeurope ".into());
        let b = Bing::new(&cfg, http.clone());
        assert_eq!(b.base, "https://api.cognitive.microsofttranslator.com");
        assert!(
            matches!(&b.mode, Mode::Azure { key, region } if key == "azure-key" && region.as_deref() == Some("westeurope"))
        );

        cfg.api_secret = Some("".into());
        let b = Bing::new(&cfg, http);
        assert!(matches!(&b.mode, Mode::Azure { region: None, .. }));
    }
}
