//! Pixel-format helpers shared by capture, rendering and the final crop.

use std::mem::MaybeUninit;

use crate::geom::{Point, Rect};

/// Bit mask that sets the alpha byte (the 4th byte in memory) of a BGRA pixel read as a
/// native-endian `u32`.
const ALPHA_MASK: u32 = u32::from_ne_bytes([0, 0, 0, 0xFF]);

/// Converts BGRX pixels (as captured by GDI, alpha undefined) to a BGRA byte vector
/// with alpha 255, in one pass and without zero-filling the destination first.
pub fn to_opaque_bgra(src: &[u32]) -> Vec<u8> {
    let len = src.len() * 4;
    let mut out = Vec::with_capacity(len);
    let written = write_opaque(&mut out.spare_capacity_mut()[..len], src);
    // SAFETY: `write_opaque` initialized the first `written` bytes (= len) of the
    // spare capacity, which is within the allocation.
    unsafe { out.set_len(written) };
    out
}

/// Writes `src` as BGRA with alpha 255 into `dst` and returns how many leading bytes of
/// `dst` it initialized (`min(dst.len() / 4, src.len()) * 4`). When `dst` is 4-byte
/// aligned (always, for a fresh large `Vec`) this is a single vectorizable OR-and-store
/// loop, i.e. memcpy speed in optimized builds.
fn write_opaque(dst: &mut [MaybeUninit<u8>], src: &[u32]) -> usize {
    let n = src.len().min(dst.len() / 4);
    let (dst, src) = (&mut dst[..n * 4], &src[..n]);
    // SAFETY: MaybeUninit<u32> has no validity requirements, so viewing the aligned
    // middle of the byte buffer as words is sound.
    let (prefix, words, _) = unsafe { dst.align_to_mut::<MaybeUninit<u32>>() };
    if prefix.is_empty() && words.len() == n {
        for (d, s) in words.iter_mut().zip(src) {
            d.write(*s | ALPHA_MASK);
        }
    } else {
        for (d, s) in dst.chunks_exact_mut(4).zip(src) {
            for (b, v) in d.iter_mut().zip((*s | ALPHA_MASK).to_ne_bytes()) {
                b.write(v);
            }
        }
    }
    n * 4
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

    fn sample_src() -> Vec<u32> {
        (0..37u8).map(|i| bgra_pixel(i, i.wrapping_mul(3), 255 - i, i)).collect()
    }

    fn assert_opaque_copy(bytes: &[u8]) {
        assert_eq!(bytes.len(), 37 * 4);
        for (i, px) in bytes.chunks_exact(4).enumerate() {
            let i = i as u8;
            assert_eq!(px, &[i, i.wrapping_mul(3), 255 - i, 255]);
        }
    }

    #[test]
    fn to_opaque_bgra_forces_alpha() {
        assert_opaque_copy(&to_opaque_bgra(&sample_src()));
        assert!(to_opaque_bgra(&[]).is_empty());
    }

    #[test]
    fn write_opaque_handles_unaligned_and_short_buffers() {
        let src = sample_src();
        for shift in 0..4 {
            let mut buf = vec![MaybeUninit::new(0u8); src.len() * 4 + shift];
            let written = write_opaque(&mut buf[shift..], &src);
            assert_eq!(written, src.len() * 4);
            // SAFETY: every byte was initialized by `vec![MaybeUninit::new(0)]`.
            let bytes: Vec<u8> = buf[shift..].iter().map(|b| unsafe { b.assume_init() }).collect();
            assert_opaque_copy(&bytes);
        }
        let mut short = vec![MaybeUninit::new(9u8); 10];
        assert_eq!(write_opaque(&mut short, &src), 8, "only whole pixels that fit");
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
