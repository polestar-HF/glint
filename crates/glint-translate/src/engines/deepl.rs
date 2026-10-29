//! DeepL API.
//!
//! `POST {host}/v2/translate` with `Authorization: DeepL-Auth-Key <key>`. The host is picked
//! from the key: Free keys end in `:fx` and use `api-free.deepl.com`, everything else uses
//! `api.deepl.com`. `base_url` overrides the host.

use serde_json::{json, Value};

use super::{non_empty, parse_err, Backend, MtOut};
use crate::http::{send, Failure, Provider, MT_TIMEOUT};
use crate::langs::{self, Role};
use crate::{EngineConfig, Result, TranslateError};

const FREE_HOST: &str = "https://api-free.deepl.com";
const PRO_HOST: &str = "https://api.deepl.com";

pub(crate) struct DeepL {
    key: String,
    host: String,
    http: reqwest::Client,
}

/// The API host for a key.
pub(crate) fn host_for_key(key: &str) -> &'static str {
    if key.trim().ends_with(":fx") {
        FREE_HOST
    } else {
        PRO_HOST
    }
}

impl DeepL {
    pub fn new(cfg: &EngineConfig, http: reqwest::Client) -> Result<Self> {
        let key = non_empty(cfg.api_key.as_deref())
            .ok_or_else(|| TranslateError::Config("The DeepL API key is not set".into()))?
            .to_string();
        let host = match non_empty(cfg.base_url.as_deref()) {
            Some(b) => b.trim_end_matches('/').to_string(),
            None => host_for_key(&key).to_string(),
        };
        Ok(DeepL { key, host, http })
    }
}

#[async_trait::async_trait]
impl Backend for DeepL {
    fn lang(&self, code: &str, role: Role) -> Result<&'static str> {
        langs::deepl(code, role)
    }

    async fn call(&self, text: &str, from: &str, to: &str) -> std::result::Result<MtOut, Failure> {
        let mut body = json!({"text": [text], "target_lang": to});
        if !from.is_empty() {
            body["source_lang"] = json!(from);
        }
        let rb = self
            .http
            .post(format!("{}/v2/translate", self.host))
            .timeout(MT_TIMEOUT)
            .header(reqwest::header::AUTHORIZATION, format!("DeepL-Auth-Key {}", self.key))
            .json(&body);
        let resp = send(rb, Provider::DeepL).await?;
        Ok(parse_response(&resp.text().await?)?)
    }
}

/// Parse `{"translations":[{"detected_source_language":"EN","text":"..."}]}`.
pub(crate) fn parse_response(body: &str) -> std::result::Result<MtOut, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    let first = v
        .pointer("/translations/0")
        .ok_or_else(|| parse_err("the response has no translation"))?;
    let text = first
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_err("the response has no translated text"))?;
    Ok(MtOut {
        text: text.to_string(),
        detected: first.get("detected_source_language").and_then(Value::as_str).map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_chosen_from_the_key_suffix() {
        assert_eq!(host_for_key("279a2e9d-83b3-c416-7e2d-f721593e42a0:fx"), "https://api-free.deepl.com");
        assert_eq!(host_for_key("  abc:fx \n"), "https://api-free.deepl.com");
        assert_eq!(host_for_key("279a2e9d-83b3-c416-7e2d-f721593e42a0"), "https://api.deepl.com");
        assert_eq!(host_for_key("abc:fx:pro"), "https://api.deepl.com");
    }

    #[test]
    fn construction_needs_a_key_and_honours_base_url() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(4);
        assert!(matches!(DeepL::new(&cfg, http.clone()), Err(TranslateError::Config(_))));
        cfg.api_key = Some(" k:fx ".into());
        let d = DeepL::new(&cfg, http.clone()).ok().expect("builds");
        assert_eq!((d.key.as_str(), d.host.as_str()), ("k:fx", "https://api-free.deepl.com"));
        cfg.base_url = Some("http://127.0.0.1:1/".into());
        assert_eq!(DeepL::new(&cfg, http).ok().expect("builds").host, "http://127.0.0.1:1");
    }

    #[test]
    fn parses_a_response() {
        let body = r#"{"translations":[{"detected_source_language":"EN","text":"Hallo, Welt!\nZweite Zeile"}]}"#;
        let out = parse_response(body).expect("parses");
        assert_eq!(out.text, "Hallo, Welt!\nZweite Zeile");
        assert_eq!(out.detected.as_deref(), Some("EN"));
        let body = r#"{"translations":[{"text":"x"}]}"#;
        assert_eq!(parse_response(body).expect("parses").detected, None);
    }

    #[test]
    fn unexpected_bodies_are_parse_errors() {
        for body in ["", "{}", r#"{"translations":[]}"#, r#"{"translations":[{}]}"#, "<html>"] {
            assert!(matches!(parse_response(body), Err(TranslateError::Parse(_))), "{body:?}");
        }
    }
}
