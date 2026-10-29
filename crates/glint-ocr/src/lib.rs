//! OCR engines and text post-processing.
//!
//! Contract: see `docs/ARCHITECTURE.md`.

use image::RgbaImage;
use serde::Serialize;

pub mod postprocess;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLine {
    pub text: String,
    /// `[x, y, w, h]` in input-image pixels.
    #[serde(rename = "box")]
    pub bbox: [f32; 4],
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub lines: Vec<OcrLine>,
    /// Lines merged into paragraphs; this is what gets translated.
    pub text: String,
    /// BCP-47-ish code guess (e.g. "en", "zh", "ja"), if known.
    pub detected_lang: Option<String>,
    pub engine: &'static str,
}

#[derive(Debug, Clone)]
pub struct OcrOptions {
    /// Merge visual lines into paragraphs (de-hyphenate, join CJK without spaces, etc.).
    pub merge_lines: bool,
    /// Language hint, e.g. "auto", "zh", "en", "ja".
    pub lang_hint: String,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self { merge_lines: true, lang_hint: "auto".into() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("model load failed: {0}")]
    Load(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("not supported on this platform")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, OcrError>;

pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &'static str;
    fn recognize(&self, img: &RgbaImage, opts: &OcrOptions) -> Result<OcrResult>;
}
