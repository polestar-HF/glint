//! Settings window: created on demand, destroyed on close (no resident webview cost).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::{hotkeys, popup};

pub const LABEL: &str = "settings";

/// Show the settings window, creating it if needed.
///
/// `WebviewWindowBuilder::build` blocks on the main thread's event loop and deadlocks on
/// Windows when it is called from the main thread itself, which is where sync commands, tray
/// menu events and the single-instance callback run. So the work always moves to a blocking
/// task.
pub fn open(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || open_now(&app));
}

fn open_now(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let mica = popup::supports_mica();
    let url = WebviewUrl::App(
        if mica {
            "settings.html"
        } else {
            "settings.html?mica=0"
        }
        .into(),
    );
    let mut builder = WebviewWindowBuilder::new(app, LABEL, url)
        .title("Glint 设置")
        .inner_size(760.0, 540.0)
        .min_inner_size(640.0, 440.0)
        .center()
        .visible(true);
    if mica {
        builder = builder.transparent(true).effects(popup::mica_effect());
    }
    let win = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            log::error!("settings window failed: {e}");
            return;
        }
    };
    // If the key recorder was listening when the window closed, give the hotkeys back.
    let app2 = app.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::Destroyed = event {
            if let Err(e) = hotkeys::set_suspended(&app2, false) {
                log::warn!("{e:#}");
            }
        }
    });
}
