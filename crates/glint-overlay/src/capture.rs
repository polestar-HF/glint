//! One-shot GDI capture of every monitor into BGRA frames.
//!
//! Per monitor: `BitBlt` (`SRCCOPY | CAPTUREBLT`) from the screen DC into a top-down
//! 32-bpp DIB section, then one vectorized copy into `MonitorFrame::bgra` with alpha
//! forced to 255. The screen DC uses virtual-desktop coordinates, so monitor rects
//! (which can be negative) are used as blit sources directly.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors, GdiFlush, GetDC,
    GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP,
    HDC, HGDIOBJ, HMONITOR, MONITORINFO, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use crate::geom::Rect;
use crate::win::{last_error, ThreadDpiScope};
use crate::{pixels, MonitorFrame, OverlayError, Result};

struct MonitorDesc {
    rect: Rect,
    work: Rect,
    scale: f32,
}

pub fn capture_all() -> Result<Vec<MonitorFrame>> {
    let _dpi = ThreadDpiScope::per_monitor_v2();
    let monitors = enumerate_monitors()?;
    if monitors.is_empty() {
        return Err(OverlayError::Capture("no monitors found".into()));
    }
    let screen = ScreenDc::get()?;
    let mem = MemDc::compatible_with(&screen)?;
    monitors.iter().map(|m| capture_monitor(&screen, &mem, m)).collect()
}

fn enumerate_monitors() -> Result<Vec<MonitorDesc>> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<HMONITOR>` passed below; EnumDisplayMonitors
        // calls back synchronously while it is alive.
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        TRUE
    }

    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: `collect` matches MONITORENUMPROC and `handles` outlives the call.
    let ok = unsafe { EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut Vec<HMONITOR> as isize)) };
    if !ok.as_bool() {
        return Err(OverlayError::Capture(last_error("EnumDisplayMonitors")));
    }

    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        // SAFETY: `info` is a properly sized MONITORINFO with cbSize set.
        if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
            log::warn!("{}", last_error("GetMonitorInfoW"));
            continue;
        }
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        // SAFETY: out-pointers are locals.
        if let Err(e) = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
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
        return Err(OverlayError::Capture(format!("empty monitor rect {:?}", m.rect)));
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
    if bits.is_null() || !bits.cast::<u32>().is_aligned() {
        return Err(OverlayError::Capture("CreateDIBSection returned unusable bits".into()));
    }

    let _selected = Selection::select(mem, &dib)?;
    // SAFETY: both DCs are live; the destination bitmap is exactly w × h.
    unsafe { BitBlt(mem.0, 0, 0, w, h, Some(screen.0), m.rect.left, m.rect.top, SRCCOPY | CAPTUREBLT) }
        .map_err(|e| OverlayError::Capture(format!("BitBlt: {e}")))?;
    // SAFETY: no preconditions. Makes sure the blit has landed before the CPU reads.
    let _ = unsafe { GdiFlush() };

    let n = w as usize * h as usize;
    // SAFETY: CreateDIBSection allocated `n` 32-bit pixels at `bits` (32-bpp rows need
    // no padding). The memory is owned by `dib`, which outlives this borrow, and the
    // pointer was checked above to be non-null and u32-aligned.
    let src = unsafe { std::slice::from_raw_parts(bits.cast::<u32>(), n) };
    let mut bgra = vec![0u8; n * 4];
    pixels::copy_opaque(&mut bgra, src);

    Ok(MonitorFrame { rect: m.rect.to_phys(), work_area: m.work.to_phys(), scale: m.scale, bgra })
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
