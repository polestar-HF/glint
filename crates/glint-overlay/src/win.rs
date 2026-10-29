//! Small Win32 helpers shared by capture and the overlay.

use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// Makes the current thread Per-Monitor-V2 DPI aware until dropped, then restores the
/// previous thread context.
///
/// The process is expected to be PMv2 already (application manifest), so this is
/// normally a no-op. It guarantees physical-pixel coordinates for this crate's calls
/// even if the caller's thread runs with another context. The process awareness is
/// never touched. Windows created inside the scope keep PMv2 for their lifetime.
pub struct ThreadDpiScope {
    previous: Option<DPI_AWARENESS_CONTEXT>,
}

impl ThreadDpiScope {
    pub fn per_monitor_v2() -> Self {
        // SAFETY: takes a predefined pseudo-handle; returns NULL on failure.
        let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            log::warn!("SetThreadDpiAwarenessContext(PMv2) failed; relying on process DPI awareness");
            Self { previous: None }
        } else {
            Self { previous: Some(previous) }
        }
    }
}

impl Drop for ThreadDpiScope {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            // SAFETY: restores the context value Windows handed us on entry.
            unsafe { SetThreadDpiAwarenessContext(previous) };
        }
    }
}

/// Formats a failed Win32 call with the thread's last error.
pub fn last_error(what: &str) -> String {
    format!("{what}: {}", windows::core::Error::from_win32())
}
