//! Settings window: created on demand, destroyed on close (no resident webview cost).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "settings";

pub fn open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
        .title("Glint 设置")
        .inner_size(760.0, 540.0)
        .min_inner_size(640.0, 440.0)
        .center()
        .visible(true)
        .build();
    if let Err(e) = built {
        log::error!("settings window failed: {e}");
    }
}
