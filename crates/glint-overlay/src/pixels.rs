//! Pixel-format helpers shared by capture, rendering and the final crop.

use crate::geom::{Point, Rect};

/// Bit mask that sets the alpha byte (the 4th byte in memory) of a BGRA pixel read as a
/// native-endian `u32`.
const ALPHA_MASK: u32 = u32::from_ne_bytes([0, 0, 0, 0xFF]);

/// Copies BGRX pixels into a BGRA byte buffer, forcing alpha to 255.
///
/// `dst.len()` must be `src.len() * 4`; extra bytes on either side are left untouched.
/// When `dst` is 4-byte aligned (always the case for a fresh `Vec<u8>` of this size in
/// practice) this compiles to a single vectorized OR-and-store pass.
pub fn copy_opaque(dst: &mut [u8], src: &[u32]) {
    let n = src.len().min(dst.len() / 4);
    let (dst, src) = (&mut dst[..n * 4], &src[..n]);
    // SAFETY: reinterpreting initialized bytes as u32 is always valid; `align_to_mut`
    // only yields the correctly aligned middle part.
    let (prefix, words, _) = unsafe { dst.align_to_mut::<u32>() };
    if prefix.is_empty() && words.len() == n {
        for (d, s) in words.iter_mut().zip(src) {
            *d = *s | ALPHA_MASK;
        }
    } else {
        for (d, s) in dst.chunks_exact_mut(4).zip(src) {
            d.copy_from_slice(&(*s | ALPHA_MASK).to_ne_bytes());
        }
    }
}

/// Crops `rect` (in frame-local pixels, clipped to the frame) out of a top-down BGRA
/// frame of `width`×`height` and returns tightly packed RGBA with alpha 255, plus the
/// clipped size.
pub fn crop_bgra_to_rgba(bgra: &[u8], width: u32, height: u32, rect: Rect) -> Option<(Vec<u8>, u32, u32)> {
    let bounds = Rect::new(0, 0, i32::try_from(width).ok()?, i32::try_from(height).ok()?);
    let r = rect.intersect(bounds)?;
    let stride = width as usize * 4;
    if bgra.len() < stride * height as usize {
        return None;
    }
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in r.top as usize..r.bottom as usize {
        let start = y * stride + r.left as usize * 4;
        let row = bgra.get(start..start + w * 4)?;
        for px in row.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], 0xFF]);
        }
    }
    Some((out, w as u32, h as u32))
}

/// The `[r, g, b]` color at `p` (frame-local) of a top-down BGRA frame.
pub fn sample_rgb(bgra: &[u8], width: u32, height: u32, p: Point) -> Option<[u8; 3]> {
    if p.x < 0 || p.y < 0 || p.x as u32 >= width || p.y as u32 >= height {
        return None;
    }
    let i = (p.y as usize * width as usize + p.x as usize) * 4;
    let px = bgra.get(i..i + 4)?;
    Some([px[2], px[1], px[0]])
}

/// `#RRGGBB`, uppercase.
pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// Relative luminance in 0..=1 (Rec. 709 weights on sRGB values; good enough to pick a
/// contrasting outline color).
pub fn luminance(rgb: [u8; 3]) -> f32 {
    (0.2126 * rgb[0] as f32 + 0.7152 * rgb[1] as f32 + 0.0722 * rgb[2] as f32) / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bgra_pixel(b: u8, g: u8, r: u8, a: u8) -> u32 {
        u32::from_ne_bytes([b, g, r, a])
    }

    #[test]
    fn copy_opaque_forces_alpha_aligned_and_unaligned() {
        let src: Vec<u32> = (0..37u8).map(|i| bgra_pixel(i, i.wrapping_mul(3), 255 - i, i)).collect();
        for shift in 0..4 {
            let mut buf = vec![0u8; src.len() * 4 + shift];
            copy_opaque(&mut buf[shift..], &src);
            for (i, px) in buf[shift..].chunks_exact(4).enumerate() {
                let i = i as u8;
                assert_eq!(px, &[i, i.wrapping_mul(3), 255 - i, 255], "shift {shift}");
            }
        }
    }

    #[test]
    fn copy_opaque_tolerates_short_buffers() {
        let src = vec![bgra_pixel(1, 2, 3, 0); 4];
        let mut dst = vec![9u8; 10];
        copy_opaque(&mut dst, &src);
        assert_eq!(dst, [1, 2, 3, 255, 1, 2, 3, 255, 9, 9]);
    }

    fn test_frame(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // B = x, G = y, R = x + y, A = 255
                v.extend_from_slice(&[x as u8, y as u8, (x + y) as u8, 255]);
            }
        }
        v
    }

    #[test]
    fn crop_swaps_channels_and_rows() {
        let f = test_frame(8, 6);
        let (px, w, h) = crop_bgra_to_rgba(&f, 8, 6, Rect::new(2, 1, 5, 3)).expect("crop");
        assert_eq!((w, h), (3, 2));
        assert_eq!(px.len(), 3 * 2 * 4);
        // First pixel: x=2, y=1 -> R=3, G=1, B=2.
        assert_eq!(&px[0..4], &[3, 1, 2, 255]);
        // Last pixel: x=4, y=2 -> R=6, G=2, B=4.
        assert_eq!(&px[20..24], &[6, 2, 4, 255]);
    }

    #[test]
    fn crop_clips_and_rejects_bad_input() {
        let f = test_frame(8, 6);
        let (_, w, h) = crop_bgra_to_rgba(&f, 8, 6, Rect::new(-3, -3, 2, 2)).expect("crop");
        assert_eq!((w, h), (2, 2));
        assert!(crop_bgra_to_rgba(&f, 8, 6, Rect::new(8, 0, 10, 2)).is_none());
        assert!(crop_bgra_to_rgba(&f[..10], 8, 6, Rect::new(0, 0, 2, 2)).is_none());
    }

    #[test]
    fn sample_and_hex() {
        let f = test_frame(8, 6);
        assert_eq!(sample_rgb(&f, 8, 6, Point::new(3, 2)), Some([5, 2, 3]));
        assert_eq!(sample_rgb(&f, 8, 6, Point::new(8, 0)), None);
        assert_eq!(sample_rgb(&f, 8, 6, Point::new(0, -1)), None);
        assert_eq!(hex([0x2F, 0x6F, 0xEB]), "#2F6FEB");
        assert!(luminance([255, 255, 255]) > 0.99);
        assert!(luminance([0, 0, 0]) < 0.01);
    }
}
