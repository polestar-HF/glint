//! Global hotkeys → capture modes.

use std::str::FromStr;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::config::Hotkeys;
use crate::pipeline::{self, CaptureMode};
use crate::AppState;

/// (Re)register all hotkeys. Invalid or conflicting chords are skipped and reported.
pub fn register(app: &AppHandle, hotkeys: &Hotkeys) -> anyhow::Result<()> {
    let gs = app.global_shortcut();
    gs.unregister_all()?;
    let state = app.state::<AppState>();
    let mut bindings = state.hotkey_bindings.lock().unwrap_or_else(|p| p.into_inner());
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
        anyhow::bail!("快捷键注册失败（可能被其他程序占用）: {}", failures.join("; "))
    }
}

pub fn handle(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let mode = {
        let state = app.state::<AppState>();
        let bindings = state.hotkey_bindings.lock().unwrap_or_else(|p| p.into_inner());
        bindings.iter().find(|(s, _)| s == shortcut).map(|(_, m)| *m)
    };
    if let Some(mode) = mode {
        pipeline::start_capture(app, mode);
    }
}
