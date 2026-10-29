//! Window snapping: a z-ordered snapshot of top-level window bounds, and hit-testing.

use crate::geom::{Point, Rect};

/// The top-most target containing `p`. `targets` must be ordered top-most first.
pub fn hit_test(targets: &[Rect], p: Point) -> Option<Rect> {
    targets.iter().copied().find(|r| r.contains(p))
}

/// Takes a snapshot of visible top-level window bounds, top-most first, in virtual-desktop
/// physical pixels. Call it before the overlay windows exist.
///
/// Skips invisible, minimized, cloaked (other virtual desktops, suspended UWP),
/// click-through, empty and own-process windows. Errors only shorten the list.
#[cfg(windows)]
pub fn snapshot() -> Vec<Rect> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

    struct Ctx {
        pid: u32,
        out: Vec<Rect>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Ctx` passed to EnumWindows below, which outlives
        // the enumeration; EnumWindows calls back synchronously on this thread.
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };
        if let Some(r) = window_bounds(hwnd, ctx.pid) {
            ctx.out.push(r);
        }
        TRUE
    }

    // SAFETY: plain query with no preconditions.
    let pid = unsafe { GetCurrentProcessId() };
    let mut ctx = Ctx { pid, out: Vec::with_capacity(64) };
    // SAFETY: `visit` matches WNDENUMPROC and `ctx` stays alive for the whole call.
    if let Err(e) = unsafe { EnumWindows(Some(visit), LPARAM(&mut ctx as *mut Ctx as isize)) } {
        log::warn!("EnumWindows failed, window snapping limited: {e}");
    }
    ctx.out
}

#[cfg(windows)]
fn window_bounds(hwnd: windows::Win32::Foundation::HWND, own_pid: u32) -> Option<Rect> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE,
        WS_EX_TRANSPARENT,
    };

    // SAFETY (whole block): every call is a read-only query on a window handle handed to
    // us by EnumWindows; a stale handle just makes the call fail, and all out-pointers
    // point at locals of the stated size.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == own_pid {
            return None;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0 {
            return None;
        }
        let mut cloaked = 0u32;
        let size = std::mem::size_of::<u32>() as u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, size).is_ok()
            && cloaked != 0
        {
            return None;
        }
        let mut r = RECT::default();
        let size = std::mem::size_of::<RECT>() as u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut c_void, size).is_err() {
            GetWindowRect(hwnd, &mut r).ok()?;
        }
        let rect = Rect::new(r.left, r.top, r.right, r.bottom);
        (!rect.is_empty()).then_some(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_respects_z_order() {
        let targets = [Rect::new(10, 10, 20, 20), Rect::new(0, 0, 100, 100)];
        assert_eq!(hit_test(&targets, Point::new(15, 15)), Some(targets[0]));
        assert_eq!(hit_test(&targets, Point::new(50, 50)), Some(targets[1]));
        assert_eq!(hit_test(&targets, Point::new(20, 20)), Some(targets[1]), "right/bottom edges are exclusive");
        assert_eq!(hit_test(&targets, Point::new(-1, 0)), None);
        assert_eq!(hit_test(&[], Point::new(0, 0)), None);
    }
}
