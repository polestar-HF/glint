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
            accent: 0x0E8CB8,
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
    select_region_with_stats(frames, opts, &mut OverlayStats::default())
}

/// Timing of one overlay run, for the app's performance statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverlayStats {
    /// When the overlay became visible on screen; `None` if it never did.
    pub shown_at: Option<std::time::Instant>,
}

/// Timing and monitor metadata of one combined capture + overlay run.
#[derive(Debug, Clone, Default)]
pub struct CaptureOverlayStats {
    /// When [`capture_all`] completed.
    pub captured_at: Option<std::time::Instant>,
    /// When the overlay became visible on screen; `None` if it never did.
    pub shown_at: Option<std::time::Instant>,
    /// `(width, height, scale)` of every captured monitor.
    pub monitors: Vec<(u32, u32, f32)>,
}

/// [`select_region`], also reporting when the overlay became visible.
pub fn select_region_with_stats(
    frames: Vec<MonitorFrame>,
    opts: &OverlayOptions,
    stats: &mut OverlayStats,
) -> Result<Outcome> {
    imp::select_region(frames, opts, stats)
}

/// Captures every monitor and snapshots snap-target windows in parallel, then opens the
/// selection overlay (reusing thread-local warm D2D/Win32 surfaces when available).
pub fn capture_and_select(
    opts: &OverlayOptions,
    stats: &mut CaptureOverlayStats,
) -> Result<Outcome> {
    imp::capture_and_select(opts, stats)
}

/// Releases cached overlay windows and GPU surfaces parked on the calling thread.
/// Call when the overlay worker has been idle to keep tray RAM low.
pub fn release_cached_resources() {
    imp::release_cached_resources()
}

/// Waits for one DWM composition frame (e.g. after hiding a popup window) before capturing.
pub fn wait_for_dwm_frame() {
    imp::wait_for_dwm_frame()
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn capture_all() -> Result<Vec<MonitorFrame>> {
        Err(OverlayError::Unsupported)
    }
    pub fn select_region(
        _: Vec<MonitorFrame>,
        _: &OverlayOptions,
        _: &mut OverlayStats,
    ) -> Result<Outcome> {
        Err(OverlayError::Unsupported)
    }
    pub fn capture_and_select(_: &OverlayOptions, _: &mut CaptureOverlayStats) -> Result<Outcome> {
        Err(OverlayError::Unsupported)
    }
    pub fn release_cached_resources() {}
    pub fn wait_for_dwm_frame() {}
}

#[cfg(windows)]
mod imp {
    use super::*;
    pub fn capture_all() -> Result<Vec<MonitorFrame>> {
        crate::capture::capture_all()
    }
    pub fn select_region(
        frames: Vec<MonitorFrame>,
        opts: &OverlayOptions,
        stats: &mut OverlayStats,
    ) -> Result<Outcome> {
        crate::overlay::select_region(frames, opts, stats)
    }
    pub fn capture_and_select(
        opts: &OverlayOptions,
        stats: &mut CaptureOverlayStats,
    ) -> Result<Outcome> {
        // Run window-snap enumeration concurrently with screen capture so DWM attribute
        // queries never add to the critical path before the overlay appears.
        let (frames, targets) = std::thread::scope(|s| {
            let snap_handle = if opts.snap_windows {
                Some(s.spawn(|| {
                    let _dpi = crate::win::ThreadDpiScope::per_monitor_v2();
                    crate::overlay::snap::snapshot()
                }))
            } else {
                None
            };
            let frames = crate::capture::capture_all();
            let targets = snap_handle.and_then(|h| h.join().ok()).unwrap_or_default();
            (frames, targets)
        });
        let frames = frames?;
        stats.captured_at = Some(std::time::Instant::now());
        stats.monitors = frames
            .iter()
            .map(|f| (f.rect.width, f.rect.height, f.scale))
            .collect();
        let mut ov = OverlayStats::default();
        let out = crate::overlay::select_region_with_targets(frames, targets, opts, &mut ov);
        stats.shown_at = ov.shown_at;
        out
    }
    pub fn release_cached_resources() {
        crate::overlay::release_cached_resources();
    }
    pub fn wait_for_dwm_frame() {
        crate::capture::wait_for_dwm_frame();
    }
}
