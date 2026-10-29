//! The current capture session's state, mirrored to the popup via events and
//! `get_session_snapshot` (so a freshly created popup can catch up).

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrPayload {
    pub session_id: u64,
    #[serde(flatten)]
    pub result: OcrResult,
    pub elapsed_ms: u64,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrError {
    pub session_id: u64,
    pub engine_id: String,
    pub message: String,
}
