//! Google Translate.
//!
//! Without an API key this uses the free web endpoint that the browser extensions use:
//! `POST {base}/translate_a/single?client=gtx&sl=..&tl=..&dt=t` with the text as a form field.
//! It needs no registration but is unofficial, so treat it as "free, best effort": Google may
//! throttle or change it at any time.
//!
//! With `api_key` set it uses the official Cloud Translation v2 API instead
//! (`POST {base}/language/translate/v2` with the `X-goog-api-key` header and a JSON body).
//!
//! `base_url`, when set, replaces the host in both modes (default
//! `https://translate.googleapis.com` for the free endpoint, and
//! `https://translation.googleapis.com` for the official one).

use serde_json::{json, Value};

use super::{non_empty, parse_err, Backend, MtOut};
use crate::http::{send, Failure, Provider, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{EngineConfig, Result, TranslateError};

const FREE_BASE: &str = "https://translate.googleapis.com";
const OFFICIAL_BASE: &str = "https://translation.googleapis.com";

pub(crate) struct Google {
    api_key: Option<String>,
    base: String,
    http: reqwest::Client,
}

impl Google {
    pub fn new(cfg: &EngineConfig, http: reqwest::Client) -> Self {
        let api_key = non_empty(cfg.api_key.as_deref()).map(str::to_owned);
        let default_base = if api_key.is_some() { OFFICIAL_BASE } else { FREE_BASE };
        let base = non_empty(cfg.base_url.as_deref()).unwrap_or(default_base);
        Google { api_key, base: base.trim_end_matches('/').to_string(), http }
    }

    async fn call_free(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        // POST, because a GET puts the text in the URL and long texts exceed its length limit.
        let rb = self
            .http
            .post(format!("{}/translate_a/single", self.base))
            .timeout(MT_TIMEOUT)
            .query(&[("client", "gtx"), ("sl", from), ("tl", to), ("dt", "t")])
            .form(&[("q", text)]);
        let body = send(rb, Provider::Google).await?.text().await?;
        Ok(parse_free(&body)?)
    }

    async fn call_official(&self, key: &str, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        let mut body = json!({"q": text, "target": to, "format": "text"});
        if from != "auto" {
            body["source"] = json!(from);
        }
        let rb = self
            .http
            .post(format!("{}/language/translate/v2", self.base))
            .timeout(MT_TIMEOUT)
            .header("X-goog-api-key", key)
            .json(&body);
        let body = send(rb, Provider::Google).await?.text().await?;
        Ok(parse_v2(&body)?)
    }
}

#[async_trait::async_trait]
impl Backend for Google {
    fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
        langs::google(code, role)
    }

    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        match &self.api_key {
            Some(key) => self.call_official(key, text, from, to).await,
            None => self.call_free(text, from, to).await,
        }
    }
}

/// Parse the free endpoint's nested array.
///
/// ```text
/// [ [ [translated, original, ...], [translated, original, ...] ],   // data[0]: one entry per sentence
///   null,
///   "en",                                                          // data[2]: detected source
///   ... ]
/// ```
/// Sentence entries carry their own trailing newlines, so concatenating them keeps the layout.
/// Entries without a string in position 0 (transliterations, nulls) are skipped.
pub(crate) fn parse_free(body: &str) -> std::result::Result<MtOut, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    let segments = v
        .get(0)
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("the response has no translation"))?;
    let text: String = segments
        .iter()
        .filter_map(|seg| seg.get(0).and_then(Value::as_str))
        .collect();
    if text.is_empty() {
        return Err(parse_err("the translation is empty"));
    }
    let detected = v
        .get(2)
        .and_then(Value::as_str)
        .or_else(|| v.pointer("/8/0/0").and_then(Value::as_str))
        .map(str::to_owned);
    Ok(MtOut { text, detected })
}

/// Parse a Cloud Translation v2 response.
pub(crate) fn parse_v2(body: &str) -> std::result::Result<MtOut, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    let first = v
        .pointer("/data/translations/0")
        .ok_or_else(|| parse_err("the response has no translation"))?;
    let text = first
        .get("translatedText")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_err("the response has no translated text"))?;
    Ok(MtOut {
        text: text.to_string(),
        detected: first.get("detectedSourceLanguage").and_then(Value::as_str).map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real responses recorded from translate.googleapis.com on 2026-10-29.
    const MULTILINE: &str = r#"[[["你好世界。\n","Hello, world.\n",null,null,10,null,null,null,[[null,true]]],["这是第二行。","This is the second line. ",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"]]]],["请保留它。\n\n","Please keep it.\n\n",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"],true]]],["- 第一项\n","- item one\n",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"],true]]],["- 第二项","- item two",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"]]]]],null,"en",null,null,null,1,[],[["en"],null,[1],["en"]]]"#;
    const FROM_CHINESE: &str = r#"[[["Hello World.\n","你好，世界。\n",null,null,10,null,null,null,[[null,true]]],["This is the second line.","这是第二行。",null,null,3,null,null,[[]],[[["af64405095a399ceb1e05c7abb7cda66","zh_en_2023q1.md"]]]]],null,"zh-CN",null,null,null,1,[],[["zh-CN"],null,[1],["zh-CN"]]]"#;
    const SAME_LANGUAGE: &str = r#"[[["This text is already English.","This text is already English.",null,null,5]],null,"en",null,null,null,1,[],[["en"],null,[1],["en"]]]"#;
    const SIMPLE: &str = r#"[[["你好","hello",null,null,10]],null,"en",null,null,null,null,[]]"#;

    #[test]
    fn multi_sentence_text_keeps_its_line_breaks() {
        let out = parse_free(MULTILINE).expect("parses");
        assert_eq!(out.text, "你好世界。\n这是第二行。请保留它。\n\n- 第一项\n- 第二项");
        assert_eq!(out.detected.as_deref(), Some("en"));
    }

    #[test]
    fn detects_the_source_language() {
        assert_eq!(parse_free(FROM_CHINESE).expect("parses").detected.as_deref(), Some("zh-CN"));
        let out = parse_free(FROM_CHINESE).expect("parses");
        assert_eq!(out.text, "Hello World.\nThis is the second line.");
        let out = parse_free(SIMPLE).expect("parses");
        assert_eq!((out.text.as_str(), out.detected.as_deref()), ("你好", Some("en")));
        assert_eq!(parse_free(SAME_LANGUAGE).expect("parses").detected.as_deref(), Some("en"));
    }

    #[test]
    fn tolerates_nulls_extra_entries_and_a_missing_detection() {
        // A transliteration entry (null in position 0), a non-array entry, and no detection.
        let body = r#"[[["a",null],[null,null,"transliteration"],"junk",["b","B"]]]"#;
        let out = parse_free(body).expect("parses");
        assert_eq!(out.text, "ab");
        assert_eq!(out.detected, None);
        // Detection only in the alternative position.
        let body = r#"[[["x","y"]],null,null,null,null,null,null,null,[["fr"]]]"#;
        assert_eq!(parse_free(body).expect("parses").detected.as_deref(), Some("fr"));
    }

    #[test]
    fn unexpected_shapes_are_parse_errors_not_panics() {
        for body in ["", "<html>captcha</html>", "null", "{}", "[]", "[null]", "[[]]", r#"[[[null]]]"#, r#"[["x"]]"#, "42"] {
            assert!(matches!(parse_free(body), Err(TranslateError::Parse(_))), "{body:?}");
        }
    }

    #[test]
    fn parses_v2_responses() {
        let body = r#"{"data":{"translations":[{"translatedText":"Bonjour","detectedSourceLanguage":"en"}]}}"#;
        let out = parse_v2(body).expect("parses");
        assert_eq!((out.text.as_str(), out.detected.as_deref()), ("Bonjour", Some("en")));
        let body = r#"{"data":{"translations":[{"translatedText":"Bonjour"}]}}"#;
        assert_eq!(parse_v2(body).expect("parses").detected, None);
        for body in ["", "{}", r#"{"data":{"translations":[]}}"#, r#"{"data":{"translations":[{}]}}"#] {
            assert!(matches!(parse_v2(body), Err(TranslateError::Parse(_))), "{body:?}");
        }
    }

    #[test]
    fn base_url_defaults_depend_on_the_mode() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(0);
        assert_eq!(Google::new(&cfg, http.clone()).base, "https://translate.googleapis.com");
        cfg.api_key = Some("k".into());
        assert_eq!(Google::new(&cfg, http.clone()).base, "https://translation.googleapis.com");
        cfg.base_url = Some("http://127.0.0.1:9/".into());
        assert_eq!(Google::new(&cfg, http).base, "http://127.0.0.1:9");
    }
}
