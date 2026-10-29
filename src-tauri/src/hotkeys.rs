//! Global hotkeys → capture modes.

use std::str::FromStr;
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::config::Hotkeys;
use crate::pipeline::{self, CaptureMode};
use crate::AppState;

/// (Re)register all hotkeys. Invalid or conflicting chords are skipped and reported.
pub fn register(app: &AppHandle, hotkeys: &Hotkeys) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    if state.hotkeys_suspended.load(Ordering::SeqCst) {
        return Ok(()); // applied when the recorder resumes
    }
    let gs = app.global_shortcut();
    gs.unregister_all()?;
    let mut bindings = state
        .hotkey_bindings
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    bindings.clear();

    let mut failures = Vec::new();
    for (chord, mode) in [
        (&hotkeys.translate, CaptureMode::Translate),
        (&hotkeys.ocr, CaptureMode::Ocr),
        (&hotkeys.silent_ocr, CaptureMode::Silent),
    ] {
        if chord.trim().is_empty() {
            continue;
        }
        let parsed = match Shortcut::from_str(chord) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{chord}: {e}"));
                continue;
            }
        };
        match gs.register(parsed) {
            Ok(()) => bindings.push((parsed, mode)),
            Err(e) => failures.push(format!("{chord}: {e}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "快捷键注册失败（可能被其他程序占用）: {}",
            failures.join("; ")
        )
    }
}

/// Temporarily release all shortcuts (while the settings key recorder listens) or restore them.
pub fn set_suspended(app: &AppHandle, suspended: bool) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let was = state.hotkeys_suspended.swap(suspended, Ordering::SeqCst);
    if was == suspended {
        return Ok(());
    }
    if suspended {
        app.global_shortcut().unregister_all()?;
        state
            .hotkey_bindings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        Ok(())
    } else {
        register(app, &state.config().hotkeys)
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyStatus {
    Ok,
    InUse,
    Invalid,
}

/// Test whether `accelerator` could be registered, without keeping it.
pub fn check(app: &AppHandle, accelerator: &str) -> HotkeyStatus {
    let Ok(shortcut) = Shortcut::from_str(accelerator) else {
        return HotkeyStatus::Invalid;
    };
    let gs = app.global_shortcut();
    // One of ours (possibly registered right now): available to us by definition.
    let state = app.state::<AppState>();
    if state
        .hotkey_bindings
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .any(|(s, _)| *s == shortcut)
        || gs.is_registered(shortcut)
    {
        return HotkeyStatus::Ok;
    }
    match gs.register(shortcut) {
        Ok(()) => {
            let _ = gs.unregister(shortcut);
            HotkeyStatus::Ok
        }
        Err(_) => HotkeyStatus::InUse,
    }
}

pub fn handle(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let mode = {
        let state = app.state::<AppState>();
        let bindings = state
            .hotkey_bindings
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        bindings
            .iter()
            .find(|(s, _)| s == shortcut)
            .map(|(_, m)| *m)
    };
    if let Some(mode) = mode {
        pipeline::start_capture(app, mode);
    }
}
