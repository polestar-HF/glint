//! Screen capture and the native region-selection overlay.
//!
//! Contract: see `docs/ARCHITECTURE.md`. Everything here is synchronous; callers run
//! `select_region` on a dedicated thread because it pumps its own message loop.
//!
//! Layout: `capture` (GDI BitBlt) and `overlay` (Win32 + Direct2D) are the Windows
//! backends. All pure logic (geometry, pixels, gestures, snap hit-testing, scene layout)
//! is platform-independent and unit-tested on any OS; on other targets it is only
//! exercised by tests, hence the `dead_code` allowance.

use image::RgbaImage;

#[cfg_attr(not(windows), allow(dead_code))]
mod geom;
#[cfg_attr(not(windows), allow(dead_code))]
mod overlay;
#[cfg_attr(not(windows), allow(dead_code))]
mod pixels;

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod win;

/// Rectangle in virtual-desktop physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// One monitor's frozen frame.
pub struct MonitorFrame {
    /// Monitor bounds in virtual-desktop physical pixels.
    pub rect: PhysRect,
    /// Monitor work area (excludes taskbar) in physical pixels.
    pub work_area: PhysRect,
    /// DPI scale factor (1.0 = 96 DPI).
    pub scale: f32,
    /// Top-down BGRA8 pixels, `rect.width * rect.height * 4` bytes, no row padding.
    pub bgra: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Translate,
    OcrOnly,
    CopyText,
}

#[derive(Debug, Clone)]
pub struct OverlayOptions {
    /// Action reported on Enter / mouse-up.
    pub default_action: Action,
    /// Accent color, 0xRRGGBB.
    pub accent: u32,
    pub snap_windows: bool,
    pub show_magnifier: bool,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            default_action: Action::Translate,
            accent: 0x1E6AC5,
            snap_windows: true,
            show_magnifier: true,
        }
    }
}

pub struct Selection {
    pub action: Action,
    /// Selected area in virtual-desktop physical pixels.
    pub rect: PhysRect,
    /// Cropped pixels of the selection (from the frozen frame).
    pub image: RgbaImage,
    /// Scale factor of the monitor containing the selection's center.
    pub scale: f32,
    /// Work area of that monitor, for popup placement.
    pub work_area: PhysRect,
}

pub enum Outcome {
    Selected(Selection),
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error("not supported on this platform")]
    Unsupported,
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("overlay failed: {0}")]
    Window(String),
}

pub type Result<T> = std::result::Result<T, OverlayError>;

/// Capture every monitor as a frozen frame. Process must be per-monitor-v2 DPI aware.
///
/// On Windows the calling thread is switched to PMv2 for the duration of the call (and
/// restored afterwards), so coordinates are physical even on a thread with another DPI
/// context. Process awareness is never changed.
pub fn capture_all() -> Result<Vec<MonitorFrame>> {
    imp::capture_all()
}

/// Show the selection overlay on all monitors and block until the user selects or cancels.
///
/// Creates its windows on the calling thread and pumps that thread's messages until
/// done; every window, graphics and GDI resource is released before it returns. A
/// `WM_QUIT` received meanwhile cancels the overlay and is re-posted for the caller.
/// Only one overlay can be open per process; a concurrent call returns
/// `OverlayError::Window`.
pub fn select_region(frames: Vec<MonitorFrame>, opts: &OverlayOptions) -> Result<Outcome> {
    imp::select_region(frames, opts)
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn capture_all() -> Result<Vec<MonitorFrame>> {
        Err(OverlayError::Unsupported)
    }
    pub fn select_region(_: Vec<MonitorFrame>, _: &OverlayOptions) -> Result<Outcome> {
        Err(OverlayError::Unsupported)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    pub fn capture_all() -> Result<Vec<MonitorFrame>> {
        crate::capture::capture_all()
    }
    pub fn select_region(frames: Vec<MonitorFrame>, opts: &OverlayOptions) -> Result<Outcome> {
        crate::overlay::select_region(frames, opts)
    }
}
