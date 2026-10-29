//! Youdao (NetEase) translation, official openapi with v3 signing.
//!
//! `POST {base}/api` (default base `https://openapi.youdao.com`) with form fields `q`, `from`,
//! `to`, `appKey`, `salt`, `sign`, `signType=v3` and `curtime`.
//!
//! `api_key` is the application ID (appKey) and `api_secret` is the application secret.
//!
//! The signature is `sha256(appKey + input + salt + curtime + appSecret)` in lower-case hex,
//! where `input` is `q` itself when it has at most 20 characters, and otherwise the first 10
//! characters, then the length in characters, then the last 10 characters.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{hex, non_empty, parse_err, unique_salt, Backend, MtOut};
use crate::http::{send, Failure, Provider, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{EngineConfig, Result, TranslateError};

const DEFAULT_BASE: &str = "https://openapi.youdao.com";
/// The documented limit for one request.
const MAX_CHARS: usize = 5000;

pub(crate) struct Youdao {
    app_key: String,
    app_secret: String,
    url: String,
    http: reqwest::Client,
}

impl Youdao {
    pub fn new(cfg: &EngineConfig, http: reqwest::Client) -> Result<Self> {
        let app_key = non_empty(cfg.api_key.as_deref())
            .ok_or_else(|| TranslateError::Config("The Youdao app ID is not set".into()))?;
        let app_secret = non_empty(cfg.api_secret.as_deref())
            .ok_or_else(|| TranslateError::Config("The Youdao app secret is not set".into()))?;
        let base = non_empty(cfg.base_url.as_deref())
            .unwrap_or(DEFAULT_BASE)
            .trim_end_matches('/');
        Ok(Youdao {
            app_key: app_key.to_string(),
            app_secret: app_secret.to_string(),
            url: format!("{base}/api"),
            http,
        })
    }
}

#[async_trait::async_trait]
impl Backend for Youdao {
    fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
        langs::youdao(code, role)
    }

    fn check_len(&self, text: &str) -> Result<()> {
        if text.chars().count() > MAX_CHARS {
            return Err(TranslateError::Service {
                status: 413,
                message: format!("The text is too long for Youdao (limit {MAX_CHARS} characters)"),
            });
        }
        Ok(())
    }

    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        let salt = unique_salt();
        let curtime = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .to_string();
        let sign = sign(&self.app_key, text, &salt, &curtime, &self.app_secret);
        let rb = self.http.post(&self.url).timeout(MT_TIMEOUT).form(&[
            ("q", text),
            ("from", from),
            ("to", to),
            ("appKey", self.app_key.as_str()),
            ("salt", salt.as_str()),
            ("sign", sign.as_str()),
            ("signType", "v3"),
            ("curtime", curtime.as_str()),
        ]);
        let resp = send(rb, Provider::Generic).await?;
        Ok(parse_response(&resp.text().await?)?)
    }
}

/// The `input` part of the signature: `q` if short, else first 10 characters, the length, and
/// the last 10 characters. Lengths and slices are counted in characters, not bytes.
pub(crate) fn truncate(q: &str) -> String {
    let chars: Vec<char> = q.chars().collect();
    let n = chars.len();
    if n <= 20 {
        return q.to_string();
    }
    let head: String = chars[..10].iter().collect();
    let tail: String = chars[n - 10..].iter().collect();
    format!("{head}{n}{tail}")
}

/// `sha256(appKey + truncate(q) + salt + curtime + appSecret)` as lower-case hex.
pub(crate) fn sign(app_key: &str, q: &str, salt: &str, curtime: &str, app_secret: &str) -> String {
    let input = truncate(q);
    let mut h = Sha256::new();
    for part in [app_key, input.as_str(), salt, curtime, app_secret] {
        h.update(part.as_bytes());
    }
    hex(&h.finalize())
}

/// Parse `{"errorCode":"0","translation":["..."],"l":"en2zh-CHS",...}`.
pub(crate) fn parse_response(body: &str) -> std::result::Result<MtOut, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    let code = match v.get("errorCode") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return Err(parse_err("the response has no status code")),
    };
    if code != "0" {
        let (status, message) = error_for_code(&code);
        return Err(TranslateError::Service { status, message });
    }
    let parts = v
        .get("translation")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("the response has no translation"))?;
    let text = parts
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return Err(parse_err("the translation is empty"));
    }
    // "en2zh-CHS": the language the text was detected (or given) as, then the target.
    let detected = v
        .get("l")
        .and_then(Value::as_str)
        .and_then(|l| l.split_once('2'))
        .map(|(from, _)| from.to_string())
        .filter(|f| !f.is_empty());
    Ok(MtOut { text, detected })
}

/// Youdao's error codes, as an (HTTP-like status, message) pair.
fn error_for_code(code: &str) -> (u16, String) {
    let (status, msg): (u16, &str) = match code {
        "101" | "104" | "105" | "106" | "107" | "109" | "113" => {
            (400, "The request was rejected as malformed")
        }
        "102" => (400, "Youdao does not support this language"),
        "103" => (413, "The text is too long"),
        "108" => (401, "Invalid app ID"),
        "110" | "112" => (
            403,
            "The app has no translation service enabled in the Youdao console",
        ),
        "111" => (401, "The Youdao developer account is not valid"),
        "201" | "202" => (401, "Signature check failed (check the app secret)"),
        "203" => (403, "This IP address is not allowed"),
        "205" => (403, "The app's platform does not match the request"),
        "206" => (
            400,
            "The request time is not valid (check the system clock)",
        ),
        "207" => (400, "The request was a repeat"),
        "301" | "302" | "303" | "304" => (500, "Youdao failed to translate"),
        "401" | "402" => (402, "Quota exceeded (out of balance)"),
        "411" | "412" => (429, "Rate limited"),
        _ => return (500, format!("Youdao error {code}")),
    };
    (status, msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected digests were computed with Python's hashlib and checked against `sha256sum`.

    #[test]
    fn truncation_leaves_short_text_alone() {
        assert_eq!(truncate(""), "");
        assert_eq!(truncate("hello"), "hello");
        assert_eq!(truncate("12345678901234567890"), "12345678901234567890"); // exactly 20
    }

    #[test]
    fn truncation_keeps_head_length_and_tail() {
        assert_eq!(truncate("123456789012345678901"), "1234567890212345678901"); // 21 chars
        assert_eq!(
            truncate("abcdefghijklmnopqrstuvwxyz"),
            "abcdefghij26qrstuvwxyz"
        );
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        let q = "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子"; // 21 characters, 63 bytes
        assert_eq!(truncate(q), "一二三四五六七八九十21乙丙丁戊己庚辛壬癸子");
        // 20 CJK characters are 60 bytes but still count as short.
        assert_eq!(
            truncate("一二三四五六七八九十甲乙丙丁戊己庚辛壬癸"),
            "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸"
        );
    }

    #[test]
    fn signature_of_short_text() {
        // sha256("appKey123hellosalt421700000000secret456")
        assert_eq!(
            sign("appKey123", "hello", "salt42", "1700000000", "secret456"),
            "6cee618d754e5c761dff09fdd37978c0943326608539a0934f865d30c4555d63"
        );
    }

    #[test]
    fn signature_of_long_text_uses_the_truncated_input() {
        // sha256("appKey123abcdefghij26qrstuvwxyzsalt421700000000secret456")
        assert_eq!(
            sign(
                "appKey123",
                "abcdefghijklmnopqrstuvwxyz",
                "salt42",
                "1700000000",
                "secret456"
            ),
            "867267ecf50b1b4f36ff498b4231c98752c50389f4e00dc6cd3710af2e1950c8"
        );
    }

    #[test]
    fn signature_of_cjk_text_and_the_20_21_boundary() {
        // sha256("appKey123一二三四五六七八九十21乙丙丁戊己庚辛壬癸子71700000001s3cr3t")
        assert_eq!(
            sign(
                "appKey123",
                "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子",
                "7",
                "1700000001",
                "s3cr3t"
            ),
            "ef0c52d38c15027c399b59d2d629a218b10655e7e468708aac48d87f9b484928"
        );
        // sha256("appKey12312345678901234567890salt421700000000secret456")
        assert_eq!(
            sign(
                "appKey123",
                "12345678901234567890",
                "salt42",
                "1700000000",
                "secret456"
            ),
            "d64725c44a04615c3ce38fcc48cbae4c77a73355318981dd3b9a6b6adbec3151"
        );
        // sha256("appKey1231234567890212345678901salt421700000000secret456")
        assert_eq!(
            sign(
                "appKey123",
                "123456789012345678901",
                "salt42",
                "1700000000",
                "secret456"
            ),
            "c18cc5983ebdf00f21e59639fb5a1fcba12016960b63ad97a429aede6c327970"
        );
    }

    #[test]
    fn parses_a_translation_and_the_detected_language() {
        let body = r#"{"errorCode":"0","query":"good","translation":["好"],"basic":{"phonetic":"gʊd"},"l":"en2zh-CHS","webdict":{"url":"x"}}"#;
        let out = parse_response(body).expect("parses");
        assert_eq!(out.text, "好");
        assert_eq!(out.detected.as_deref(), Some("en"));
    }

    #[test]
    fn multiple_translation_entries_join_with_newlines() {
        let body = r#"{"errorCode":"0","translation":["第一行","第二行"],"l":"zh-CHS2en"}"#;
        let out = parse_response(body).expect("parses");
        assert_eq!(out.text, "第一行\n第二行");
        assert_eq!(out.detected.as_deref(), Some("zh-CHS"));
    }

    #[test]
    fn error_codes_become_short_messages() {
        let err = |code: &str| match parse_response(&format!(r#"{{"errorCode":"{code}"}}"#)) {
            Err(TranslateError::Service { status, message }) => (status, message),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(err("108"), (401, "Invalid app ID".to_string()));
        assert_eq!(err("202").0, 401);
        assert!(err("202").1.contains("app secret"));
        assert_eq!(
            err("401"),
            (402, "Quota exceeded (out of balance)".to_string())
        );
        assert_eq!(err("411"), (429, "Rate limited".to_string()));
        assert_eq!(err("103").0, 413);
        assert_eq!(err("9999"), (500, "Youdao error 9999".to_string()));
        // Numeric codes are accepted too.
        assert!(matches!(
            parse_response(r#"{"errorCode":108}"#),
            Err(TranslateError::Service { status: 401, .. })
        ));
    }

    #[test]
    fn unexpected_bodies_are_parse_errors() {
        for body in [
            "",
            "{}",
            "<html>",
            r#"{"errorCode":"0"}"#,
            r#"{"errorCode":"0","translation":[]}"#,
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
        let mut cfg = crate::default_engines().remove(5);
        assert!(
            matches!(Youdao::new(&cfg, http.clone()), Err(TranslateError::Config(m)) if m.contains("app ID"))
        );
        cfg.api_key = Some("k".into());
        assert!(
            matches!(Youdao::new(&cfg, http.clone()), Err(TranslateError::Config(m)) if m.contains("secret"))
        );
        cfg.api_secret = Some("s".into());
        assert_eq!(
            Youdao::new(&cfg, http.clone())
                .unwrap_or_else(|e| panic!("builds: {e}"))
                .url,
            "https://openapi.youdao.com/api"
        );
        cfg.base_url = Some("http://127.0.0.1:1/".into());
        assert_eq!(
            Youdao::new(&cfg, http)
                .unwrap_or_else(|e| panic!("builds: {e}"))
                .url,
            "http://127.0.0.1:1/api"
        );
    }

    #[test]
    fn long_text_is_refused_with_a_clear_message() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(5);
        cfg.api_key = Some("k".into());
        cfg.api_secret = Some("s".into());
        let y = Youdao::new(&cfg, http).unwrap_or_else(|e| panic!("builds: {e}"));
        assert!(y.check_len(&"a".repeat(5000)).is_ok());
        assert!(matches!(
            y.check_len(&"a".repeat(5001)),
            Err(TranslateError::Service { status: 413, .. })
        ));
    }
}
