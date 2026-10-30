//! IPC commands (docs/ARCHITECTURE.md → "IPC").

use std::sync::atomic::Ordering;

use glint_translate::{EngineConfig, TranslateRequest};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::config::{self, Config};
use crate::pipeline::{self, CaptureMode};
use crate::session::SessionSnapshot;
use crate::{hotkeys, perf, popup, settings, tray, AppState};

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Config {
    state.config()
}

#[tauri::command]
pub fn save_config(app: AppHandle, state: State<'_, AppState>, config: Config) -> CmdResult<()> {
    let previous = state.config();
    config::save(&state.config_path, &config).map_err(|e| e.to_string())?;
    state.set_config(config.clone());

    // A conflicting hotkey is reported to the caller, but must not skip the rest of the save.
    let hotkeys_result = if previous.hotkeys != config.hotkeys {
        let res = hotkeys::register(&app, &config.hotkeys).map_err(|e| e.to_string());
        tray::refresh(&app, res.as_ref().err().map(String::as_str));
        res
    } else {
        Ok(())
    };
    if previous.general.autostart != config.general.autostart {
        let al = app.autolaunch();
        let res = if config.general.autostart {
            al.enable()
        } else {
            al.disable()
        };
        if let Err(e) = res {
            log::warn!("autostart toggle failed: {e}");
        }
    }
    let _ = app.emit("config:changed", &config);
    hotkeys_result
}

#[tauri::command]
pub fn get_session_snapshot(state: State<'_, AppState>) -> Option<SessionSnapshot> {
    state.session_snapshot()
}

#[tauri::command]
pub fn retranslate(app: AppHandle, text: String, source_lang: String, target_lang: String) -> u64 {
    pipeline::retranslate(&app, text, source_lang, target_lang)
}

#[tauri::command]
pub fn retranslate_engine(
    app: AppHandle,
    session_id: u64,
    engine_id: String,
    text: String,
    source_lang: String,
    target_lang: String,
) {
    pipeline::retranslate_engine(&app, session_id, engine_id, text, source_lang, target_lang);
}

#[tauri::command]
pub fn check_hotkey(app: AppHandle, accelerator: String) -> hotkeys::HotkeyStatus {
    hotkeys::check(&app, &accelerator)
}

#[tauri::command]
pub fn hotkeys_suspend(app: AppHandle, suspended: bool) -> CmdResult<()> {
    hotkeys::set_suspended(&app, suspended).map_err(|e| e.to_string())
}

/// Why an engine test failed; `code` lets the UI localize it (see `TranslateError::code`).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineTestError {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_secs: Option<u64>,
}

impl From<glint_translate::TranslateError> for EngineTestError {
    fn from(e: glint_translate::TranslateError) -> Self {
        Self {
            code: e.code(),
            message: e.user_message(),
            retry_after_secs: e.retry_after().map(|d| d.as_secs().max(1)),
        }
    }
}

/// Bypasses the cache and any rate-limit cool-down: the user asked for a real request.
#[tauri::command]
pub async fn test_engine(
    state: State<'_, AppState>,
    engine: EngineConfig,
) -> Result<String, EngineTestError> {
    let translator = glint_translate::build(&engine, &state.http)?;
    let target = state.config().translate.target_lang;
    let req = TranslateRequest {
        text: "Hello, world.".into(),
        source_lang: "en".into(),
        target_lang: if target.starts_with("en") {
            "zh-Hans".into()
        } else {
            target
        },
        secondary_lang: None,
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    Ok(translator.translate(&req, tx).await?)
}

#[tauri::command]
pub fn copy_text(app: AppHandle, text: String) -> CmdResult<()> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn popup_set_pinned(state: State<'_, AppState>, pinned: bool) {
    state.popup_pinned.store(pinned, Ordering::Relaxed);
}

#[tauri::command]
pub fn popup_hide(app: AppHandle) {
    popup::hide(&app);
}

#[tauri::command]
pub fn popup_resize(app: AppHandle, height: f64) {
    popup::resize(&app, height);
}

#[tauri::command]
pub fn start_capture(app: AppHandle, window: tauri::Window, mode: String) {
    let mode = if mode == "ocr" {
        CaptureMode::Ocr
    } else {
        CaptureMode::Translate
    };
    // When Settings asked, get it out of the frozen frame (the popup hides itself in
    // `pipeline::start_capture`). The minimize animation needs a moment before BitBlt.
    if window.label() == settings::LABEL {
        let _ = window.minimize();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            pipeline::start_capture(&app, mode);
        });
        return;
    }
    pipeline::start_capture(&app, mode);
}

#[tauri::command]
pub fn get_perf_stats(app: AppHandle, state: State<'_, AppState>) -> perf::PerfStats {
    state.perf.stats(system_info(&app, &state))
}

#[tauri::command]
pub fn reset_perf_stats(state: State<'_, AppState>) {
    state.perf.reset();
}

#[tauri::command]
pub fn get_diagnostics(app: AppHandle, state: State<'_, AppState>) -> String {
    let stats = state.perf.stats(system_info(&app, &state));
    let cfg = state.config();
    let mut report = perf::diagnostics(&stats);
    // Settings that change the numbers; never keys or endpoints.
    let engines: Vec<String> = cfg
        .translate
        .engines
        .iter()
        .filter(|e| e.enabled)
        .map(|e| format!("{}({})", e.id, e.kind))
        .collect();
    report.push_str(&format!(
        "\nOCR 引擎: {}  合并段落: {}\n已启用翻译: {}\n",
        cfg.ocr.engine,
        cfg.ocr.merge_lines,
        engines.join(", ")
    ));
    report
}

fn system_info(app: &AppHandle, state: &AppState) -> perf::SystemInfo {
    #[cfg(windows)]
    let os = format!("Windows build {}", popup::windows_build());
    #[cfg(not(windows))]
    let os = std::env::consts::OS.to_string();
    perf::SystemInfo {
        app: app.package_info().version.to_string(),
        os,
        cpu_threads: std::thread::available_parallelism().map_or(0, |n| n.get() as u32),
        webview: tauri::webview_version().unwrap_or_else(|_| "-".into()),
        monitors: state.perf.monitors(),
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    settings::open(&app);
}
