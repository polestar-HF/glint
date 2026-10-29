//! The fullscreen region-selection overlay.
//!
//! - `input`: gesture/keyboard state machine (pure).
//! - `snap`: window-snap hit-testing (pure) and the Win32 window snapshot.
//! - `layout`: what to draw for a view, as primitives (pure).
//! - `render`: Direct2D/DirectWrite execution of those primitives (Windows).
//! - `window`: window class, per-monitor popups and the message loop (Windows).

pub(crate) mod input;
pub(crate) mod layout;
pub(crate) mod snap;

#[cfg(windows)]
mod render;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub(crate) use window::select_region;

use crate::geom::Rect;
use crate::{pixels, Action, MonitorFrame, OverlayError, Result, Selection};
use image::RgbaImage;

/// Checks the frames handed to `select_region`: at least one, and each buffer exactly
/// `width * height * 4` bytes of a non-empty monitor.
pub(crate) fn validate_frames(frames: &[MonitorFrame]) -> Result<()> {
    if frames.is_empty() {
        return Err(OverlayError::Window("no monitor frames".into()));
    }
    for (i, f) in frames.iter().enumerate() {
        let expected = (f.rect.width as usize)
            .checked_mul(f.rect.height as usize)
            .and_then(|n| n.checked_mul(4));
        let fits_i32 = i32::try_from(f.rect.width).is_ok() && i32::try_from(f.rect.height).is_ok();
        if f.rect.width == 0 || f.rect.height == 0 || !fits_i32 || expected != Some(f.bgra.len()) {
            return Err(OverlayError::Window(format!(
                "frame {i}: {}x{} with {} bytes of pixels",
                f.rect.width,
                f.rect.height,
                f.bgra.len()
            )));
        }
    }
    Ok(())
}

/// Crops the selected `rect` (virtual-desktop px) out of monitor `monitor`'s frozen frame.
pub(crate) fn build_selection(frames: &[MonitorFrame], rect: Rect, monitor: usize, action: Action) -> Result<Selection> {
    let frame = frames
        .get(monitor)
        .ok_or_else(|| OverlayError::Window(format!("selection on unknown monitor {monitor}")))?;
    let origin = (frame.rect.x, frame.rect.y);
    let local = rect.offset(-origin.0, -origin.1);
    let (rgba, w, h) = pixels::crop_bgra_to_rgba(&frame.bgra, frame.rect.width, frame.rect.height, local)
        .ok_or_else(|| OverlayError::Window(format!("selection {rect:?} is outside its monitor")))?;
    let clipped = Rect::new(0, 0, frame.rect.width as i32, frame.rect.height as i32)
        .intersect(local)
        .unwrap_or(local)
        .offset(origin.0, origin.1);
    let image = RgbaImage::from_raw(w, h, rgba)
        .ok_or_else(|| OverlayError::Window("cropped buffer has the wrong size".into()))?;
    Ok(Selection { action, rect: clipped.to_phys(), image, scale: frame.scale, work_area: frame.work_area })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhysRect;

    fn frame(x: i32, y: i32, w: u32, h: u32) -> MonitorFrame {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for py in 0..h {
            for px in 0..w {
                bgra.extend_from_slice(&[px as u8, py as u8, 0xAA, 255]);
            }
        }
        MonitorFrame {
            rect: PhysRect { x, y, width: w, height: h },
            work_area: PhysRect { x, y, width: w, height: h - 4 },
            scale: 1.5,
            bgra,
        }
    }

    #[test]
    fn validate_rejects_bad_frames() {
        assert!(validate_frames(&[]).is_err());
        assert!(validate_frames(&[frame(0, 0, 8, 8)]).is_ok());
        let mut bad = frame(0, 0, 8, 8);
        bad.bgra.pop();
        assert!(validate_frames(&[bad]).is_err());
        let mut empty = frame(0, 0, 8, 8);
        empty.rect.width = 0;
        empty.bgra.clear();
        assert!(validate_frames(&[empty]).is_err());
    }

    #[test]
    fn selection_on_negative_monitor() {
        let frames = vec![frame(0, 0, 16, 16), frame(-16, -8, 16, 16)];
        let rect = Rect::new(-12, -6, -8, -2); // local (4, 2) .. (8, 6)
        let sel = build_selection(&frames, rect, 1, Action::OcrOnly).expect("selection");
        assert_eq!(sel.rect, PhysRect { x: -12, y: -6, width: 4, height: 4 });
        assert_eq!(sel.image.dimensions(), (4, 4));
        // Top-left pixel: local (4, 2) -> B=4, G=2, R=0xAA -> RGBA (0xAA, 2, 4, 255).
        assert_eq!(sel.image.get_pixel(0, 0).0, [0xAA, 2, 4, 255]);
        assert_eq!(sel.scale, 1.5);
        assert_eq!(sel.work_area, frames[1].work_area);
        assert_eq!(sel.action, Action::OcrOnly);
    }

    #[test]
    fn selection_is_clipped_and_validated() {
        let frames = vec![frame(0, 0, 16, 16)];
        let sel = build_selection(&frames, Rect::new(10, 10, 40, 40), 0, Action::Translate).expect("selection");
        assert_eq!(sel.rect, PhysRect { x: 10, y: 10, width: 6, height: 6 });
        assert!(build_selection(&frames, Rect::new(20, 20, 30, 30), 0, Action::Translate).is_err());
        assert!(build_selection(&frames, Rect::new(0, 0, 4, 4), 3, Action::Translate).is_err());
    }
}
