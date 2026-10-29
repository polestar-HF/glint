//! Baidu Translate (fanyi), official general-text API with MD5 signing.
//!
//! `POST {base}/api/trans/vip/translate` (default base `https://fanyi-api.baidu.com`) with form
//! fields `q`, `from`, `to`, `appid`, `salt` and `sign`.
//!
//! `api_key` is the APP ID and `api_secret` is the secret key.
//!
//! The signature is `md5(appid + q + salt + secret)` in lower-case hex, computed over the raw
//! UTF-8 text before URL-encoding. Baidu translates each line of `q` separately and returns
//! one `{src, dst}` pair per line, which is joined back with newlines.

use md5::{Digest, Md5};
use serde_json::Value;

use super::{hex, non_empty, parse_err, unique_salt, Backend, MtOut};
use crate::http::{send, Failure, Provider, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{EngineConfig, Result, TranslateError};

const DEFAULT_BASE: &str = "https://fanyi-api.baidu.com";
/// Baidu asks for requests of at most 6000 bytes of UTF-8.
const MAX_BYTES: usize = 6000;

pub(crate) struct Baidu {
    app_id: String,
    secret: String,
    url: String,
    http: reqwest::Client,
}

impl Baidu {
    pub fn new(cfg: &EngineConfig, http: reqwest::Client) -> Result<Self> {
        let app_id = non_empty(cfg.api_key.as_deref())
            .ok_or_else(|| TranslateError::Config("The Baidu APP ID is not set".into()))?;
        let secret = non_empty(cfg.api_secret.as_deref())
            .ok_or_else(|| TranslateError::Config("The Baidu secret key is not set".into()))?;
        let base = non_empty(cfg.base_url.as_deref())
            .unwrap_or(DEFAULT_BASE)
            .trim_end_matches('/');
        Ok(Baidu {
            app_id: app_id.to_string(),
            secret: secret.to_string(),
            url: format!("{base}/api/trans/vip/translate"),
            http,
        })
    }
}

#[async_trait::async_trait]
impl Backend for Baidu {
    fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
        langs::baidu(code, role)
    }

    fn check_len(&self, text: &str) -> Result<()> {
        if text.len() > MAX_BYTES {
            return Err(TranslateError::Service {
                status: 413,
                message: format!("The text is too long for Baidu (limit about {MAX_BYTES} bytes)"),
            });
        }
        Ok(())
    }

    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        let salt = unique_salt();
        let sign = sign(&self.app_id, text, &salt, &self.secret);
        let rb = self.http.post(&self.url).timeout(MT_TIMEOUT).form(&[
            ("q", text),
            ("from", from),
            ("to", to),
            ("appid", self.app_id.as_str()),
            ("salt", salt.as_str()),
            ("sign", sign.as_str()),
        ]);
        let resp = send(rb, Provider::Generic).await?;
        Ok(parse_response(&resp.text().await?)?)
    }
}

/// `md5(appid + q + salt + secret)` as lower-case hex.
pub(crate) fn sign(app_id: &str, q: &str, salt: &str, secret: &str) -> String {
    let mut h = Md5::new();
    for part in [app_id, q, salt, secret] {
        h.update(part.as_bytes());
    }
    hex(&h.finalize())
}

/// Parse `{"from":"en","to":"zh","trans_result":[{"src":"..","dst":".."}]}` or
/// `{"error_code":"54001","error_msg":"Invalid Sign"}`.
pub(crate) fn parse_response(body: &str) -> std::result::Result<MtOut, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    if let Some(code) = v.get("error_code") {
        let code = match code {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // 52000 is "success" in Baidu's tables, but it comes with a result, not as an error.
        if code != "52000" {
            let (status, message) = error_for_code(&code);
            return Err(TranslateError::Service { status, message });
        }
    }
    let lines = v
        .get("trans_result")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("the response has no translation"))?;
    let dst: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.get("dst").and_then(Value::as_str))
        .collect();
    let text = dst.join("\n");
    if text.trim().is_empty() {
        return Err(parse_err("the translation is empty"));
    }
    Ok(MtOut {
        text,
        detected: v.get("from").and_then(Value::as_str).map(str::to_owned),
    })
}

/// Baidu's error codes, as an (HTTP-like status, message) pair.
fn error_for_code(code: &str) -> (u16, String) {
    let (status, msg): (u16, &str) = match code {
        "52001" => (504, "Baidu timed out"),
        "52002" => (500, "Baidu had a system error"),
        "52003" => (
            401,
            "Invalid APP ID, or the translation service is not enabled for it",
        ),
        "54000" => (400, "A required parameter is missing"),
        "54001" => (401, "Signature check failed (check the secret key)"),
        "54003" => (429, "Rate limited"),
        "54004" => (402, "Quota exceeded (account balance)"),
        "54005" => (429, "Rate limited (too many long requests)"),
        "58000" => (403, "This IP address is not allowed"),
        "58001" => (400, "Baidu does not support this language"),
        "58002" => (403, "The translation service is closed for this account"),
        "90107" => (401, "Authentication failed"),
        _ => return (500, format!("Baidu error {code}")),
    };
    (status, msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_the_documented_example() {
        // From Baidu's API documentation: appid 2015063000000001, q "apple", salt 1435660288,
        // secret 12345678.
        assert_eq!(
            sign("2015063000000001", "apple", "1435660288", "12345678"),
            "f89f9594663708c1605f3d736d01d2d4"
        );
    }

    #[test]
    fn signature_is_over_the_raw_utf8_text() {
        // md5("20151113000005349你好，世界\n第二行1435660288osubCEzlGjzvw8qdQc41"), computed with
        // Python's hashlib.
        assert_eq!(
            sign(
                "20151113000005349",
                "你好，世界\n第二行",
                "1435660288",
                "osubCEzlGjzvw8qdQc41"
            ),
            "efe2e411cc8e8e1efe93c13c4a0cfe49"
        );
    }

    #[test]
    fn parses_one_result_per_line() {
        let body = r#"{"from":"en","to":"zh","trans_result":[{"src":"Hello, world.","dst":"你好，世界。"},{"src":"","dst":""},{"src":"Second line","dst":"第二行"}]}"#;
        let out = parse_response(body).expect("parses");
        assert_eq!(out.text, "你好，世界。\n\n第二行");
        assert_eq!(out.detected.as_deref(), Some("en"));
    }

    #[test]
    fn error_codes_become_short_messages() {
        let err = |body: &str| match parse_response(body) {
            Err(TranslateError::Service { status, message }) => (status, message),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            err(r#"{"error_code":"54001","error_msg":"Invalid Sign"}"#).0,
            401
        );
        assert!(err(r#"{"error_code":"54001","error_msg":"Invalid Sign"}"#)
            .1
            .contains("secret key"));
        assert_eq!(
            err(r#"{"error_code":"54003","error_msg":"Invalid Access Limit"}"#),
            (429, "Rate limited".to_string())
        );
        assert_eq!(
            err(r#"{"error_code":"54004","error_msg":"Please recharge"}"#).0,
            402
        );
        assert_eq!(
            err(r#"{"error_code":"52003","error_msg":"UNAUTHORIZED USER"}"#).0,
            401
        );
        assert_eq!(
            err(r#"{"error_code":"52001","error_msg":"TIMEOUT"}"#).0,
            504
        );
        assert_eq!(
            err(r#"{"error_code":"12345"}"#),
            (500, "Baidu error 12345".to_string())
        );
        // Numeric codes work too.
        assert_eq!(err(r#"{"error_code":54003}"#).0, 429);
    }

    #[test]
    fn unexpected_bodies_are_parse_errors() {
        for body in [
            "",
            "{}",
            "<html>",
            r#"{"trans_result":[]}"#,
            r#"{"trans_result":[{"src":"a"}]}"#,
        ] {
            assert!(
                matches!(parse_response(body), Err(TranslateError::Parse(_))),
                "{body:?}"
            );
        }
    }

    #[test]
    fn construction_needs_both_credentials() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(6);
        assert!(
            matches!(Baidu::new(&cfg, http.clone()), Err(TranslateError::Config(m)) if m.contains("APP ID"))
        );
        cfg.api_key = Some("id".into());
        assert!(
            matches!(Baidu::new(&cfg, http.clone()), Err(TranslateError::Config(m)) if m.contains("secret"))
        );
        cfg.api_secret = Some("s".into());
        assert_eq!(
            Baidu::new(&cfg, http.clone())
                .unwrap_or_else(|e| panic!("builds: {e}"))
                .url,
            "https://fanyi-api.baidu.com/api/trans/vip/translate"
        );
        cfg.base_url = Some("http://127.0.0.1:1/".into());
        assert_eq!(
            Baidu::new(&cfg, http)
                .unwrap_or_else(|e| panic!("builds: {e}"))
                .url,
            "http://127.0.0.1:1/api/trans/vip/translate"
        );
    }

    #[test]
    fn long_text_is_refused_by_byte_count() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(6);
        cfg.api_key = Some("id".into());
        cfg.api_secret = Some("s".into());
        let b = Baidu::new(&cfg, http).unwrap_or_else(|e| panic!("builds: {e}"));
        assert!(b.check_len(&"a".repeat(6000)).is_ok());
        assert!(b.check_len(&"a".repeat(6001)).is_err());
        // 2001 CJK characters are 6003 bytes.
        assert!(b.check_len(&"你".repeat(2001)).is_err());
        assert!(b.check_len(&"你".repeat(2000)).is_ok());
    }
}
