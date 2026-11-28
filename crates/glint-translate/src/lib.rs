//! Translation engines.
//!
//! Contract: see `docs/ARCHITECTURE.md`.
//!
//! Typical wiring in the app:
//!
//! ```text
//! let http = glint_translate::http_client();           // connect timeout 5 s, rustls
//! let cache = glint_translate::Cache::default();        // share one for the whole app
//! let engines: Vec<Box<dyn Translator>> = cfg.engines.iter()
//!     .filter(|e| e.enabled)
//!     .filter_map(|e| glint_translate::build_cached(e, &http, &cache).ok())
//!     .collect();
//! glint_translate::translate_all(&engines, &req, |ev| { /* emit tr:* events */ }).await;
//! ```
//!
//! Every engine honours `EngineConfig::base_url`, which makes proxies and local mocks easy.
//! Timeouts: connect 5 s (set by [`http_client`]), total 30 s for machine-translation engines
//! and 60 s for LLM engines (set per request). Each request is retried at most once, and only
//! for connection errors and HTTP 5xx, and only while nothing has streamed yet. HTTP 429 is
//! never retried: the engine instead cools down (see [`Cache`]), honouring `Retry-After`.

mod cache;
mod cooldown;
mod engines;
mod http;
pub mod langs;
mod layout;
mod prompt;
mod run;
mod sse;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

pub use cache::{Cache, CacheKey};
pub use http::http_client;
pub use run::{translate_all, EngineEvent};

/// Receives incremental text as it streams in.
pub type DeltaSink = UnboundedSender<String>;

#[derive(Debug, Clone)]
pub struct TranslateRequest {
    pub text: String,
    /// "auto" or a BCP-47 code (see the [`langs`] module).
    pub source_lang: String,
    pub target_lang: String,
    /// Used when the source is "auto" (or equals the target) and the text turns out to
    /// already be in `target_lang`: the text is then translated into this language instead.
    /// `None` means "return the text unchanged".
    pub secondary_lang: Option<String>,
}

impl TranslateRequest {
    /// A request with no secondary language.
    pub fn new(
        text: impl Into<String>,
        source_lang: impl Into<String>,
        target_lang: impl Into<String>,
    ) -> Self {
        Self {
            text: text.into(),
            source_lang: source_lang.into(),
            target_lang: target_lang.into(),
            secondary_lang: None,
        }
    }
}

impl Default for TranslateRequest {
    fn default() -> Self {
        Self::new(String::new(), "auto", "zh-Hans")
    }
}

/// Persisted per-engine settings. Stored in the app config as an array.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    /// Unique instance id, e.g. "claude", "deepl-1".
    pub id: String,
    /// Engine kind, e.g. "anthropic", "openai", "google", "bing", "deepl", "youdao", "baidu".
    pub kind: String,
    pub enabled: bool,
    /// Display name override.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    /// Secondary secret (e.g. Youdao app secret, Baidu secret key).
    #[serde(default)]
    pub api_secret: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Extra instructions appended to LLM prompts.
    #[serde(default)]
    pub prompt: Option<String>,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("enabled", &self.enabled)
            .field("name", &self.name)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field(
                "api_secret",
                &self.api_secret.as_ref().map(|_| "[redacted]"),
            )
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("prompt", &self.prompt)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    #[error("missing configuration: {0}")]
    Config(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("service error ({status}): {message}")]
    Service { status: u16, message: String },
    #[error("unexpected response: {0}")]
    Parse(String),
    /// The service said "too many requests" (HTTP 429), or the engine is still cooling down
    /// after it did. `retry_after` is how long until it will be asked again.
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after: Option<std::time::Duration>,
    },
}

impl TranslateError {
    /// A short sentence that is fit to show in the UI, without the technical prefix that
    /// `Display` adds (for example `Invalid API key` instead of `service error (401): ...`).
    pub fn user_message(&self) -> String {
        match self {
            TranslateError::Config(m) | TranslateError::Network(m) => m.clone(),
            TranslateError::Service { message, .. } => message.clone(),
            TranslateError::Parse(m) => format!("Unexpected response from the service ({m})"),
            TranslateError::RateLimited { message, .. } => message.clone(),
        }
    }

    /// A stable category the UI localizes and attaches advice to:
    /// `rate_limited`, `quota`, `auth`, `not_found`, `too_long`, `bad_request`, `server`,
    /// `timeout`, `network`, `config`, `bad_response` or `failed`.
    pub fn code(&self) -> &'static str {
        match self {
            TranslateError::Config(_) => "config",
            TranslateError::Network(m) if m.contains("timed out") => "timeout",
            TranslateError::Network(_) => "network",
            TranslateError::Parse(_) => "bad_response",
            TranslateError::RateLimited { .. } => "rate_limited",
            TranslateError::Service { status, message } => match status {
                429 if message.starts_with("Quota") => "quota",
                429 => "rate_limited",
                402 | 456 => "quota",
                401 => "auth",
                403 if message.contains("API key") => "auth",
                404 => "not_found",
                413 => "too_long",
                400 | 403 | 422 => "bad_request",
                408 => "timeout",
                500..=599 => "server",
                _ => "failed",
            },
        }
    }

    /// For rate limiting: how long until the engine is tried again.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            TranslateError::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, TranslateError>;

#[async_trait::async_trait]
pub trait Translator: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// Translate `req`, sending incremental text to `deltas`; returns the full translation.
    async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String>;
}

/// Build the engine described by `cfg`. Fails with [`TranslateError::Config`] for an unknown
/// kind or missing credentials. Nothing is sent over the network until `translate` is called.
pub fn build(cfg: &EngineConfig, http: &reqwest::Client) -> Result<Box<dyn Translator>> {
    engines::build(cfg, http)
}

/// [`build`], wrapped so that repeated identical requests are answered from `cache`.
pub fn build_cached(
    cfg: &EngineConfig,
    http: &reqwest::Client,
    cache: &Cache,
) -> Result<Box<dyn Translator>> {
    Ok(cache.wrap(build(cfg, http)?, cfg))
}

/// [`build`], wrapped for an explicit user retry: bypasses cached results and any active local
/// rate-limit cool-down, clearing the cool-down if the retry succeeds.
pub fn build_retry(
    cfg: &EngineConfig,
    http: &reqwest::Client,
    cache: &Cache,
) -> Result<Box<dyn Translator>> {
    Ok(cache.wrap_retry(build(cfg, http)?, cfg))
}

/// The default engine list, in display order: Google and Bing enabled, the rest disabled
/// until the user adds credentials.
pub fn default_engines() -> Vec<EngineConfig> {
    let mk = |id: &str, kind: &str, enabled: bool, model: Option<&str>| EngineConfig {
        id: id.into(),
        kind: kind.into(),
        enabled,
        name: None,
        api_key: None,
        api_secret: None,
        base_url: None,
        model: model.map(str::to_owned),
        prompt: None,
    };
    vec![
        mk("google", "google", true, None),
        mk("bing", "bing", true, None),
        mk(
            "claude",
            "anthropic",
            false,
            Some(engines::DEFAULT_ANTHROPIC_MODEL),
        ),
        mk(
            "openai",
            "openai",
            false,
            Some(engines::DEFAULT_OPENAI_MODEL),
        ),
        mk("deepl", "deepl", false, None),
        mk("youdao", "youdao", false, None),
        mk("baidu", "baidu", false, None),
    ]
}

/// The display name an engine of this kind gets when `EngineConfig::name` is not set.
pub fn default_name(kind: &str) -> &'static str {
    engines::default_name(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_engine_list_matches_the_contract() {
        let list = default_engines();
        let ids: Vec<_> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            ["google", "bing", "claude", "openai", "deepl", "youdao", "baidu"]
        );
        let enabled: Vec<_> = list.iter().map(|e| e.enabled).collect();
        assert_eq!(enabled, [true, true, false, false, false, false, false]);
        assert_eq!(list[2].kind, "anthropic");
        assert_eq!(list[2].model.as_deref(), Some("claude-haiku-5-5"));
    }

    #[test]
    fn keyless_defaults_build_and_keyed_ones_report_missing_config() {
        let http = http_client();
        for cfg in default_engines() {
            let built = build(&cfg, &http);
            match cfg.kind.as_str() {
                "google" | "bing" => {
                    let t = built.expect("keyless engines build without credentials");
                    assert_eq!(t.id(), cfg.id);
                }
                _ => assert!(
                    matches!(built, Err(TranslateError::Config(_))),
                    "{} should need credentials",
                    cfg.kind
                ),
            }
        }
    }

    #[test]
    fn unknown_kind_is_a_config_error() {
        let mut cfg = default_engines().remove(0);
        cfg.kind = "nope".into();
        let err = build(&cfg, &http_client()).err().expect("must fail");
        assert!(matches!(err, TranslateError::Config(_)));
    }

    #[test]
    fn user_message_drops_the_technical_prefix() {
        let e = TranslateError::Service {
            status: 401,
            message: "Invalid API key".into(),
        };
        assert_eq!(e.to_string(), "service error (401): Invalid API key");
        assert_eq!(e.user_message(), "Invalid API key");
    }

    #[test]
    fn engine_config_round_trips_camel_case() {
        let json =
            r#"{"id":"c","kind":"anthropic","enabled":true,"apiKey":"k","baseUrl":"http://x"}"#;
        let cfg: EngineConfig = serde_json::from_str(json).expect("parse");
        assert_eq!(cfg.api_key.as_deref(), Some("k"));
        assert_eq!(cfg.base_url.as_deref(), Some("http://x"));
        assert!(cfg.model.is_none());
    }

    #[test]
    fn engine_config_debug_redacts_secrets() {
        let mut cfg = default_engines().remove(0);
        cfg.api_key = Some("super-secret-key".into());
        cfg.api_secret = Some("super-secret-value".into());
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("super-secret-key"), "{dbg}");
        assert!(!dbg.contains("super-secret-value"), "{dbg}");
        assert!(dbg.contains("[redacted]"), "{dbg}");
    }
}
