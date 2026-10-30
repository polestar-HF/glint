//! The current capture session's state, mirrored to the popup via events and
//! `get_session_snapshot` (so a freshly created popup can catch up).

use std::time::Instant;

use glint_ocr::OcrResult;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Translate,
    Ocr,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Streaming,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrState {
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<OcrPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationState {
    pub engine_id: String,
    pub engine_name: String,
    pub status: Status,
    pub text: String,
    /// Number of `tr:delta` events already folded into `text` (lets the popup drop replays).
    pub delta_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Error category for localized text (`glint_translate::TranslateError::code`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// While a rate-limited engine cools down: seconds until it is asked again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Engine start to first streamed text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    #[serde(skip)]
    pub started: Option<Instant>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: u64,
    pub mode: Mode,
    pub source_lang: String,
    pub target_lang: String,
    pub ocr: OcrState,
    pub translations: Vec<TranslationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timings: Option<CaptureTimings>,
}

/// Hotkey → overlay timings of a capture session (milliseconds).
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTimings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotkey_to_overlay_ms: Option<u64>,
    pub capture_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay_ms: Option<u64>,
}

/// Where the OCR time went (milliseconds). `load_ms` only when this capture loaded the model.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrTimings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub det_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rec_ms: Option<u64>,
}

impl SessionSnapshot {
    pub fn new(session_id: u64, mode: Mode, source_lang: String, target_lang: String) -> Self {
        Self {
            session_id,
            mode,
            source_lang,
            target_lang,
            ocr: OcrState {
                status: Status::Pending,
                result: None,
                message: None,
            },
            translations: Vec::new(),
            timings: None,
        }
    }

    pub fn translation_mut(&mut self, engine_id: &str) -> Option<&mut TranslationState> {
        self.translations
            .iter_mut()
            .find(|t| t.engine_id == engine_id)
    }
}

// ---- Event payloads (names and shapes per docs/ARCHITECTURE.md) ----

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStart {
    pub session_id: u64,
    pub mode: Mode,
    pub source_lang: String,
    pub target_lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timings: Option<CaptureTimings>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrPayload {
    pub session_id: u64,
    #[serde(flatten)]
    pub result: OcrResult,
    pub elapsed_ms: u64,
    pub timings: OcrTimings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionError {
    pub session_id: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrStart {
    pub session_id: u64,
    pub engine_id: String,
    pub engine_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrDelta {
    pub session_id: u64,
    pub engine_id: String,
    pub delta: String,
    /// 0-based index per engine per session.
    pub seq: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrDone {
    pub session_id: u64,
    pub engine_id: String,
    pub text: String,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrError {
    pub session_id: u64,
    pub engine_id: String,
    pub message: String,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}
