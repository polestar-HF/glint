//! System tray icon and menu.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::pipeline::{self, CaptureMode};
use crate::{settings, AppState};

const ID: &str = "main";

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let hk = app.state::<AppState>().config().hotkeys;
    let label = |text: &str, chord: &str| {
        if chord.is_empty() {
            text.to_string()
        } else {
            format!("{text}\t{chord}")
        }
    };
    let translate = MenuItem::with_id(
        app,
        "translate",
        label("截图翻译", &hk.translate),
        true,
        None::<&str>,
    )?;
    let ocr = MenuItem::with_id(app, "ocr", label("截图识字", &hk.ocr), true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &translate,
            &ocr,
            &PredefinedMenuItem::separator(app)?,
            &settings_item,
            &quit,
        ],
    )
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(ID)
        .tooltip("Glint")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "translate" => pipeline::start_capture(app, CaptureMode::Translate),
            "ocr" => pipeline::start_capture(app, CaptureMode::Ocr),
            "settings" => settings::open(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                pipeline::start_capture(tray.app_handle(), CaptureMode::Translate);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuild the menu so its hotkey labels match the current config, and surface a hotkey
/// registration problem in the tooltip (the tray is the only UI that is always present).
pub fn refresh(app: &AppHandle, hotkey_error: Option<&str>) {
    let Some(tray) = app.tray_by_id(ID) else {
        return;
    };
    match build_menu(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => log::warn!("tray menu rebuild failed: {e}"),
    }
    let tooltip = match hotkey_error {
        Some(err) => format!("Glint — {err}"),
        None => "Glint".to_string(),
    };
    let _ = tray.set_tooltip(Some(tooltip));
}
