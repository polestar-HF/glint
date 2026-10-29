//! IPC commands (docs/ARCHITECTURE.md → "IPC").

use std::sync::atomic::Ordering;

use glint_translate::{EngineConfig, TranslateRequest};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::config::{self, Config};
use crate::pipeline::{self, CaptureMode};
use crate::session::SessionSnapshot;
use crate::{hotkeys, popup, settings, AppState};

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

    if previous.hotkeys != config.hotkeys {
        hotkeys::register(&app, &config.hotkeys).map_err(|e| e.to_string())?;
    }
    if previous.general.autostart != config.general.autostart {
        let al = app.autolaunch();
        let res = if config.general.autostart { al.enable() } else { al.disable() };
        if let Err(e) = res {
            log::warn!("autostart toggle failed: {e}");
        }
    }
    let _ = app.emit("config:changed", &config);
    Ok(())
}

#[tauri::command]
pub fn get_session_snapshot(state: State<'_, AppState>) -> Option<SessionSnapshot> {
    state.session_snapshot()
}

#[tauri::command]
pub fn retranslate(
    app: AppHandle,
    text: String,
    source_lang: String,
    target_lang: String,
) -> u64 {
    pipeline::retranslate(&app, text, source_lang, target_lang)
}

#[tauri::command]
pub async fn test_engine(state: State<'_, AppState>, engine: EngineConfig) -> CmdResult<String> {
    let translator =
        glint_translate::build(&engine, &state.http).map_err(|e| e.user_message())?;
    let target = state.config().translate.target_lang;
    let req = TranslateRequest {
        text: "Hello, world.".into(),
        source_lang: "en".into(),
        target_lang: if target.starts_with("en") { "zh-Hans".into() } else { target },
        secondary_lang: None,
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    translator.translate(&req, tx).await.map_err(|e| e.user_message())
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
pub fn start_capture(app: AppHandle, mode: String) {
    let mode = if mode == "ocr" { CaptureMode::Ocr } else { CaptureMode::Translate };
    // Let the invoking window (settings) get out of the way of the frozen frame.
    if let Some(w) = app.get_webview_window(settings::LABEL) {
        let _ = w.minimize();
    }
    pipeline::start_capture(&app, mode);
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    settings::open(&app);
}
