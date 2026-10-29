//! OCR engines and text post-processing.
//!
//! Contract: see `docs/ARCHITECTURE.md`.
//!
//! * [`PaddleOcr`]: PP-OCR (det + rec ONNX models) on ONNX Runtime, cross-platform.
//! * `WindowsOcr` (Windows only): `Windows.Media.Ocr`.
//! * [`postprocess`]: reading order, paragraph merging, normalization and language guess, shared
//!   by both engines.

use image::RgbaImage;
use serde::Serialize;

mod imgproc;
mod lang;
mod mem;
mod paddle;
pub mod postprocess;
#[cfg(windows)]
mod windows_ocr;

pub use mem::trim_process_memory;
pub use paddle::{PaddleOcr, PaddleOptions, Timings};
#[cfg(windows)]
pub use windows_ocr::WindowsOcr;

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
        Self {
            merge_lines: true,
            lang_hint: "auto".into(),
        }
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

    /// [`OcrEngine::recognize`], plus per-stage timings when the engine measures them.
    fn recognize_with_timings(
        &self,
        img: &RgbaImage,
        opts: &OcrOptions,
    ) -> Result<(OcrResult, Option<Timings>)> {
        self.recognize(img, opts).map(|r| (r, None))
    }
}
