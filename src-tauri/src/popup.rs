//! Result popup window: lazily created, placed next to the selection, auto-sized to its
//! content by the page (`popup_resize`), hidden on blur, destroyed after being idle.

use std::sync::atomic::Ordering;
use std::time::Duration;

use glint_overlay::PhysRect;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

use crate::AppState;

pub const LABEL: &str = "popup";
const WIDTH: f64 = 420.0;
const INITIAL_HEIGHT: f64 = 168.0;
const GAP: f64 = 8.0;
const MAX_SCREEN_FRACTION: f64 = 0.7;

/// Where the popup should appear: the selection and the monitor it lives on.
#[derive(Debug, Clone, Copy)]
pub struct Anchor {
    pub rect: PhysRect,
    pub work_area: PhysRect,
    pub scale: f64,
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn ensure(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = window(app) {
        return Ok(w);
    }
    let mica = supports_mica();
    let url = WebviewUrl::App(
        if mica {
            "popup.html"
        } else {
            "popup.html?mica=0"
        }
        .into(),
    );
    let mut builder = WebviewWindowBuilder::new(app, LABEL, url)
        .title("Glint")
        .inner_size(WIDTH, INITIAL_HEIGHT)
        .min_inner_size(300.0, 96.0)
        .decorations(false)
        .resizable(true)
        .maximizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(true)
        .visible(false)
        .focused(true);
    if mica {
        builder = builder.transparent(true).effects(mica_effect());
    }
    let win = builder.build()?;

    let app2 = app.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let state = app2.state::<AppState>();
            let close_on_blur = state.config().ui.close_on_blur;
            if close_on_blur && !state.popup_pinned.load(Ordering::Relaxed) {
                hide(&app2);
            }
        }
    });
    Ok(win)
}

/// Show the popup next to `anchor`. Creates the webview on first use.
pub fn show_at(app: &AppHandle, anchor: Anchor) {
    let state = app.state::<AppState>();
    *state.popup_anchor.lock().unwrap_or_else(|p| p.into_inner()) = Some(anchor);
    state.popup_hide_gen.fetch_add(1, Ordering::Relaxed);

    let win = match ensure(app) {
        Ok(w) => w,
        Err(e) => {
            log::error!("popup create failed: {e}");
            return;
        }
    };
    let height = win
        .inner_size()
        .map(|s| s.height as f64)
        .unwrap_or(INITIAL_HEIGHT * anchor.scale)
        .max(INITIAL_HEIGHT * anchor.scale);
    let (x, y, w, h) = place(anchor, WIDTH * anchor.scale, height);
    // Position first: moving across monitors may trigger a DPI rescale that we then override.
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let _ = win.set_size(PhysicalSize::new(w, h));
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
}

pub fn hide(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    let _ = win.hide();
    schedule_destroy(app);
}

pub fn is_visible(app: &AppHandle) -> bool {
    window(app)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// The page reports its content height (logical px); grow/shrink to fit, staying on-screen.
pub fn resize(app: &AppHandle, content_height: f64) {
    let Some(win) = window(app) else { return };
    let state = app.state::<AppState>();
    let anchor = *state.popup_anchor.lock().unwrap_or_else(|p| p.into_inner());
    let scale = win.scale_factor().unwrap_or(1.0);
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let Ok(size) = win.inner_size() else { return };

    let (wa_y, wa_h) = match anchor {
        Some(a) => (a.work_area.y, a.work_area.height as f64),
        None => (pos.y, 10_000.0),
    };
    let h = (content_height * scale)
        .clamp(96.0 * scale, wa_h * MAX_SCREEN_FRACTION)
        .round();
    let mut y = pos.y;
    let bottom_limit = wa_y as f64 + wa_h;
    if y as f64 + h > bottom_limit {
        y = (bottom_limit - h).max(wa_y as f64) as i32;
    }
    if (h - size.height as f64).abs() < 1.0 && y == pos.y {
        return;
    }
    let _ = win.set_size(PhysicalSize::new(size.width as f64, h));
    if y != pos.y {
        let _ = win.set_position(PhysicalPosition::new(pos.x, y));
    }
}

fn schedule_destroy(app: &AppHandle) {
    let state = app.state::<AppState>();
    let secs = state.config().ui.popup_idle_destroy_secs;
    if secs == 0 {
        return;
    }
    let generation = state.popup_hide_gen.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let state = app.state::<AppState>();
        if state.popup_hide_gen.load(Ordering::Relaxed) != generation || is_visible(&app) {
            return;
        }
        if let Some(win) = window(&app) {
            log::info!("destroying idle popup webview");
            let _ = win.destroy();
        }
    });
}

/// Choose the popup rectangle (physical px): below the selection, else above, else to the
/// right/left, finally clamped into the work area.
pub fn place(a: Anchor, w: f64, h: f64) -> (f64, f64, f64, f64) {
    let gap = GAP * a.scale;
    let wa = a.work_area;
    let (wa_l, wa_t) = (wa.x as f64, wa.y as f64);
    let (wa_r, wa_b) = (wa_l + wa.width as f64, wa_t + wa.height as f64);
    let (sel_l, sel_t) = (a.rect.x as f64, a.rect.y as f64);
    let (sel_r, sel_b) = (sel_l + a.rect.width as f64, sel_t + a.rect.height as f64);
    let h = h.min(wa.height as f64 * MAX_SCREEN_FRACTION);

    let (mut x, mut y) = if sel_b + gap + h <= wa_b {
        (sel_l, sel_b + gap)
    } else if sel_t - gap - h >= wa_t {
        (sel_l, sel_t - gap - h)
    } else if sel_r + gap + w <= wa_r {
        (sel_r + gap, sel_t)
    } else if sel_l - gap - w >= wa_l {
        (sel_l - gap - w, sel_t)
    } else {
        (sel_r - w, sel_b - h)
    };
    x = x.clamp(wa_l, (wa_r - w).max(wa_l));
    y = y.clamp(wa_t, (wa_b - h).max(wa_t));
    (x.round(), y.round(), w.round(), h.round())
}

pub fn mica_effect() -> tauri::utils::config::WindowEffectsConfig {
    tauri::utils::config::WindowEffectsConfig {
        effects: vec![tauri::window::Effect::Mica],
        ..Default::default()
    }
}

/// Mica needs Windows 11 (build 22000+); Windows 10 gets the page's solid fallback.
pub fn supports_mica() -> bool {
    #[cfg(windows)]
    {
        windows_build() >= 22000
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn windows_build() -> u32 {
    // RtlGetVersion is not subject to manifest-based version lies.
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a properly sized, writable OSVERSIONINFOW.
    if unsafe { RtlGetVersion(&mut info) }.is_ok() {
        info.dwBuildNumber
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(x: i32, y: i32, w: u32, h: u32) -> Anchor {
        Anchor {
            rect: PhysRect {
                x,
                y,
                width: w,
                height: h,
            },
            work_area: PhysRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            scale: 1.0,
        }
    }

    #[test]
    fn places_below_when_room() {
        let (x, y, _, _) = place(anchor(100, 100, 300, 50), 420.0, 200.0);
        assert_eq!((x, y), (100.0, 158.0));
    }

    #[test]
    fn places_above_near_bottom() {
        let (_, y, _, h) = place(anchor(100, 900, 300, 100), 420.0, 200.0);
        assert_eq!(y, 900.0 - 8.0 - h);
    }

    #[test]
    fn full_screen_selection_stays_inside() {
        let (x, y, w, h) = place(anchor(0, 0, 1920, 1040), 420.0, 300.0);
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 1920.0 && y + h <= 1040.0);
    }

    #[test]
    fn clamps_to_right_edge() {
        let (x, _, w, _) = place(anchor(1800, 100, 100, 40), 420.0, 200.0);
        assert!(x + w <= 1920.0);
    }
}
