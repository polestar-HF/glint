//! One-shot GDI capture of every monitor into BGRX/BGRA frames.
//!
//! Per monitor: `BitBlt` (`SRCCOPY | CAPTUREBLT`) from the screen DC into a top-down
//! 32-bpp DIB section, then one direct byte copy into `MonitorFrame::bgra`. Multi-monitor
//! setups capture every monitor in parallel. The screen DC uses virtual-desktop coordinates,
//! so monitor rects (which can be negative) are used as blit sources directly.
//!
//! Alpha byte note: GDI leaves the 4th byte undefined, which is fine because Direct2D uses
//! `D2D1_ALPHA_MODE_IGNORE`, the magnifier samples only B/G/R, and `crop_bgra_to_rgba` forces
//! alpha to 255 on the cropped selection.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors,
    GdiFlush, GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use crate::geom::Rect;
use crate::win::{last_error, ThreadDpiScope};
use crate::{MonitorFrame, OverlayError, Result};

struct MonitorDesc {
    rect: Rect,
    work: Rect,
    scale: f32,
}

/// Wait for one DWM composition frame (typically 6–16 ms) so a just-hidden window is no
/// longer present in the screen DC before capturing.
pub fn wait_for_dwm_frame() {
    // SAFETY: DwmFlush has no preconditions.
    if unsafe { DwmFlush() }.is_err() {
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

pub fn capture_all() -> Result<Vec<MonitorFrame>> {
    let _dpi = ThreadDpiScope::per_monitor_v2();
    let monitors = enumerate_monitors()?;
    if monitors.is_empty() {
        return Err(OverlayError::Capture("no monitors found".into()));
    }
    if monitors.len() == 1 {
        let screen = ScreenDc::get()?;
        let mem = MemDc::compatible_with(&screen)?;
        return Ok(vec![capture_monitor(&screen, &mem, &monitors[0])?]);
    }
    // Multi-monitor: capture every monitor concurrently so dual-2K / 4K setups pay for one
    // monitor's BitBlt + readback instead of the sum.
    std::thread::scope(|s| {
        let handles: Vec<_> = monitors
            .iter()
            .map(|m| {
                s.spawn(move || {
                    let _dpi = ThreadDpiScope::per_monitor_v2();
                    let screen = ScreenDc::get()?;
                    let mem = MemDc::compatible_with(&screen)?;
                    capture_monitor(&screen, &mem, m)
                })
            })
            .collect();
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let frame = h
                .join()
                .map_err(|_| OverlayError::Capture("capture worker panicked".into()))??;
            out.push(frame);
        }
        Ok(out)
    })
}

fn enumerate_monitors() -> Result<Vec<MonitorDesc>> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed below; EnumDisplayMonitors
        // calls back synchronously while it is alive.
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        TRUE
    }

    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: `collect` matches MONITORENUMPROC and `handles` outlives the call.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    if !ok.as_bool() {
        return Err(OverlayError::Capture(last_error("EnumDisplayMonitors")));
    }

    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `info` is a properly sized MONITORINFO with cbSize set.
        if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
            log::warn!("{}", last_error("GetMonitorInfoW"));
            continue;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        // SAFETY: out-pointers are locals.
        if let Err(e) =
            unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        {
            log::warn!("GetDpiForMonitor failed, assuming 96 DPI: {e}");
            dpi_x = 96;
        }
        let r = info.rcMonitor;
        let w = info.rcWork;
        out.push(MonitorDesc {
            rect: Rect::new(r.left, r.top, r.right, r.bottom),
            work: Rect::new(w.left, w.top, w.right, w.bottom),
            scale: dpi_x.max(1) as f32 / 96.0,
        });
    }
    Ok(out)
}

fn capture_monitor(screen: &ScreenDc, mem: &MemDc, m: &MonitorDesc) -> Result<MonitorFrame> {
    let (w, h) = (m.rect.width(), m.rect.height());
    if w == 0 || h == 0 {
        return Err(OverlayError::Capture(format!(
            "empty monitor rect {:?}",
            m.rect
        )));
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // negative height: top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = null_mut();
    // SAFETY: `bmi` describes a 32-bpp DIB; `bits` receives the pixel pointer.
    let dib = unsafe { CreateDIBSection(Some(screen.0), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
        .map_err(|e| OverlayError::Capture(format!("CreateDIBSection {w}x{h}: {e}")))?;
    let dib = Bitmap(dib);
    if bits.is_null() {
        return Err(OverlayError::Capture(
            "CreateDIBSection returned unusable bits".into(),
        ));
    }

    let _selected = Selection::select(mem, &dib)?;
    // SAFETY: both DCs are live; the destination bitmap is exactly w × h.
    unsafe {
        BitBlt(
            mem.0,
            0,
            0,
            w,
            h,
            Some(screen.0),
            m.rect.left,
            m.rect.top,
            SRCCOPY | CAPTUREBLT,
        )
    }
    .map_err(|e| OverlayError::Capture(format!("BitBlt: {e}")))?;
    // SAFETY: no preconditions. Makes sure the blit has landed before the CPU reads.
    let _ = unsafe { GdiFlush() };

    let byte_len = w as usize * h as usize * 4;
    // SAFETY: CreateDIBSection allocated `byte_len` bytes at `bits` (32-bpp rows need no
    // padding), owned by `dib` which outlives this slice. Direct memcpy avoids touching alpha
    // (consumers use D2D1_ALPHA_MODE_IGNORE or overwrite alpha on crop).
    let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), byte_len) }.to_vec();

    Ok(MonitorFrame {
        rect: m.rect.to_phys(),
        work_area: m.work.to_phys(),
        scale: m.scale,
        bgra,
    })
}

/// The screen DC, released on drop.
struct ScreenDc(HDC);

impl ScreenDc {
    fn get() -> Result<Self> {
        // SAFETY: GetDC(NULL) returns the DC of the whole virtual screen.
        let dc = unsafe { GetDC(None) };
        if dc.is_invalid() {
            return Err(OverlayError::Capture("GetDC(NULL) failed".into()));
        }
        Ok(Self(dc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: releases the DC obtained with GetDC(NULL).
        unsafe { ReleaseDC(None, self.0) };
    }
}

/// A memory DC, deleted on drop.
struct MemDc(HDC);

impl MemDc {
    fn compatible_with(screen: &ScreenDc) -> Result<Self> {
        // SAFETY: `screen` is a live DC.
        let dc = unsafe { CreateCompatibleDC(Some(screen.0)) };
        if dc.is_invalid() {
            return Err(OverlayError::Capture("CreateCompatibleDC failed".into()));
        }
        Ok(Self(dc))
    }
}

impl Drop for MemDc {
    fn drop(&mut self) {
        // SAFETY: deletes the DC we created; any selected bitmap was deselected first.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// A GDI bitmap, deleted on drop.
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: deletes the bitmap we created; it is no longer selected into a DC.
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

/// Keeps a bitmap selected into a DC, restoring the previous object on drop.
struct Selection<'a> {
    dc: &'a MemDc,
    previous: HGDIOBJ,
}

impl<'a> Selection<'a> {
    fn select(dc: &'a MemDc, bitmap: &Bitmap) -> Result<Self> {
        // SAFETY: both handles are live and owned by us.
        let previous = unsafe { SelectObject(dc.0, bitmap.0.into()) };
        if previous.is_invalid() {
            return Err(OverlayError::Capture("SelectObject failed".into()));
        }
        Ok(Self { dc, previous })
    }
}

impl Drop for Selection<'_> {
    fn drop(&mut self) {
        // SAFETY: re-selects the DC's original bitmap so ours can be deleted.
        unsafe { SelectObject(self.dc.0, self.previous) };
    }
}
