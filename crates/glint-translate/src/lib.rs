//! Translation engines.
//!
//! Contract: see `docs/ARCHITECTURE.md`.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

/// Receives incremental text as it streams in.
pub type DeltaSink = UnboundedSender<String>;

#[derive(Debug, Clone)]
pub struct TranslateRequest {
    pub text: String,
    /// "auto" or a language code (see `langs` module once added).
    pub source_lang: String,
    pub target_lang: String,
}

/// Persisted per-engine settings. Stored in the app config as an array.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

pub type Result<T> = std::result::Result<T, TranslateError>;

#[async_trait::async_trait]
pub trait Translator: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// Translate `req`, sending incremental text to `deltas`; returns the full translation.
    async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String>;
}
