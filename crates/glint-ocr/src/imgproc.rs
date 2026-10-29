//! Small image helpers shared by the engines. Everything here is pure and unit-tested.
//!
//! Resampling uses `image`'s separable filters (Lanczos3 when enlarging, Triangle when
//! shrinking). `fast_image_resize` would be 5-10x faster on large inputs, but the pipeline only
//! resamples recognition crops (a few kilopixels) and, for very large selections, the detector
//! input once; measured cost is a few milliseconds, so the extra dependency is not worth it.

use image::{
    imageops::{self, FilterType},
    Rgb, RgbImage, RgbaImage,
};

/// Drops the alpha channel. Fully transparent and fully opaque pixels are taken as they are
/// (GDI captures leave alpha at 0); partially transparent ones are blended over white.
pub(crate) fn rgba_to_rgb(img: &RgbaImage) -> RgbImage {
    let (w, h) = img.dimensions();
    let mut out = RgbImage::new(w, h);
    for (s, d) in img
        .as_raw()
        .chunks_exact(4)
        .zip(out.as_mut().chunks_exact_mut(3))
    {
        let a = s[3] as u32;
        if a == 0 || a == 255 {
            d.copy_from_slice(&s[..3]);
        } else {
            for k in 0..3 {
                d[k] = ((s[k] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
            }
        }
    }
    out
}

/// Per-channel median of the outer two-pixel ring: the page/background colour of a crop.
pub(crate) fn background_color(img: &RgbImage) -> [u8; 3] {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return [255, 255, 255];
    }
    let ring = 2.min(w.div_ceil(2)).min(h.div_ceil(2));
    let mut hist = [[0u32; 256]; 3];
    let mut n = 0u32;
    let mut add = |p: &Rgb<u8>| {
        for (c, hc) in hist.iter_mut().enumerate() {
            hc[p.0[c] as usize] += 1;
        }
        n += 1;
    };
    for y in 0..h {
        if y < ring || y >= h - ring {
            for x in 0..w {
                add(img.get_pixel(x, y));
            }
        } else {
            for x in (0..ring).chain(w - ring..w) {
                add(img.get_pixel(x, y));
            }
        }
    }
    let mut out = [0u8; 3];
    for (c, hc) in hist.iter().enumerate() {
        let mut acc = 0;
        for (v, &cnt) in hc.iter().enumerate() {
            acc += cnt;
            if acc * 2 >= n {
                out[c] = v as u8;
                break;
            }
        }
    }
    out
}

pub(crate) fn round_up(v: u32, m: u32) -> u32 {
    v.div_ceil(m) * m
}

/// Places `img` in the middle of a `cw` x `ch` canvas filled with `bg`; returns the canvas and
/// the offset of the image inside it. `cw`/`ch` must not be smaller than the image.
pub(crate) fn centered_canvas(
    img: &RgbImage,
    cw: u32,
    ch: u32,
    bg: [u8; 3],
) -> (RgbImage, (u32, u32)) {
    let (w, h) = img.dimensions();
    let (cw, ch) = (cw.max(w), ch.max(h));
    let mut canvas = RgbImage::from_pixel(cw, ch, Rgb(bg));
    let off = ((cw - w) / 2, (ch - h) / 2);
    imageops::replace(&mut canvas, img, off.0 as i64, off.1 as i64);
    (canvas, off)
}

/// Resizes with a filter that suits the direction: Lanczos3 to enlarge (sharp, preserves thin
/// strokes), Triangle to shrink (area-aware, no ringing on text).
pub(crate) fn resize(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    let (sw, sh) = img.dimensions();
    if (sw, sh) == (w, h) {
        return img.clone();
    }
    let enlarging = (w as u64 * h as u64) >= (sw as u64 * sh as u64);
    imageops::resize(
        img,
        w.max(1),
        h.max(1),
        if enlarging {
            FilterType::Lanczos3
        } else {
            FilterType::Triangle
        },
    )
}

/// `v / 127.5 - 1` for every byte value.
fn norm_lut() -> [f32; 256] {
    let mut lut = [0f32; 256];
    for (v, o) in lut.iter_mut().enumerate() {
        *o = v as f32 / 127.5 - 1.0;
    }
    lut
}

/// Writes `img` into a planar `[3, plane_h, stride]` float buffer in B, G, R order (PP-OCR was
/// trained on OpenCV BGR images) normalized with mean 0.5 / std 0.5. `dst` must be zeroed
/// beyond the image if the image is narrower than `stride`.
pub(crate) fn write_bgr_planar(dst: &mut [f32], plane_len: usize, stride: usize, img: &RgbImage) {
    let lut = norm_lut();
    let (w, h) = (img.width() as usize, img.height() as usize);
    debug_assert!(w <= stride && h * stride <= plane_len && dst.len() >= 3 * plane_len);
    let raw = img.as_raw();
    for y in 0..h {
        let row = &raw[y * w * 3..(y + 1) * w * 3];
        let o = y * stride;
        let (b, rest) = dst[o..].split_at_mut(plane_len);
        let (g, r) = rest.split_at_mut(plane_len);
        for (x, p) in row.chunks_exact(3).enumerate() {
            b[x] = lut[p[2] as usize];
            g[x] = lut[p[1] as usize];
            r[x] = lut[p[0] as usize];
        }
    }
}

/// Snaps the vertical extent of an axis-aligned text box to the ink inside it.
///
/// Detection at 1x localizes a line only to about +-1.5 px, and for text that the selection
/// clips (no margin) the DB core can sit several pixels off-centre, which makes the
/// recognition crop cut ink or leave the text hugging one edge. Recognition is sensitive to that,
/// so the box is re-centred on the rows that actually contain strokes (horizontal gradient
/// energy, which also works on gradients and photos), growing or shrinking by at most
/// `MAX_SHIFT` px per side, with a margin of 20 % of the ink height. Returns `(y0, y1)`; the
/// input is returned unchanged if no clear ink band is found.
pub(crate) fn refine_vertical(img: &RgbImage, x0: f32, y0: f32, x1: f32, y1: f32) -> (f32, f32) {
    const MAX_SHIFT: f32 = 4.0;
    let (w, h) = img.dimensions();
    let xa = (x0.floor().max(0.0) as u32).min(w.saturating_sub(2));
    let xb = (x1.ceil().max(0.0) as u32).min(w);
    let lo = ((y0 - MAX_SHIFT).floor().max(0.0)) as u32;
    let hi = ((y1 + MAX_SHIFT).ceil().max(0.0) as u32).min(h);
    if xb <= xa + 4 || hi <= lo + 4 {
        return (y0, y1);
    }
    let luma = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        (299 * p[0] as i32 + 587 * p[1] as i32 + 114 * p[2] as i32) / 1000
    };
    let energy: Vec<f32> = (lo..hi)
        .map(|y| {
            (xa..xb - 1)
                .map(|x| (luma(x + 1, y) - luma(x, y)).abs() as f32)
                .sum::<f32>()
                / (xb - xa) as f32
        })
        .collect();
    let peak = energy.iter().cloned().fold(0.0f32, f32::max);
    if peak < 2.0 {
        return (y0, y1);
    }
    let thr = 0.12 * peak;
    let n = energy.len() as i32;
    // Seed at the strongest row inside the original box.
    let (sy0, sy1) = (
        ((y0 - lo as f32).max(0.0)) as i32,
        ((y1 - lo as f32).ceil() as i32).min(n),
    );
    let Some(seed) = (sy0..sy1.max(sy0 + 1))
        .filter(|&i| i < n)
        .max_by(|&a, &b| energy[a as usize].total_cmp(&energy[b as usize]))
    else {
        return (y0, y1);
    };
    // Grow while rows still carry ink; two quiet rows in a row end the band.
    let (mut top, mut bot) = (seed, seed);
    let quiet = |i: i32| i < 0 || i >= n || energy[i as usize] < thr;
    while !(quiet(top - 1) && quiet(top - 2)) && top > 0 {
        top -= 1;
    }
    while !(quiet(bot + 1) && quiet(bot + 2)) && bot < n - 1 {
        bot += 1;
    }
    while top < bot && quiet(top) {
        top += 1;
    }
    while bot > top && quiet(bot) {
        bot -= 1;
    }
    let ink_h = (bot - top + 1) as f32;
    let margin = (0.2 * ink_h).clamp(1.5, 6.0);
    let ny0 = (lo as f32 + top as f32 - margin).max(0.0);
    let ny1 = (lo as f32 + bot as f32 + 1.0 + margin).min(h as f32);
    if (ny0 - y0).abs() > 2.0 * MAX_SHIFT || (ny1 - y1).abs() > 2.0 * MAX_SHIFT || ny1 - ny0 < 4.0 {
        return (y0, y1);
    }
    (ny0, ny1)
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Cuts the text line out of `img`. `quad` is `[tl, tr, br, bl]`.
///
/// Axis-aligned quads are cropped directly. Tilted ones are resampled bilinearly into an upright
/// rectangle. A crop that is much taller than wide (rotated text) is turned 90 degrees
/// counter-clockwise so it reads left to right, as PaddleOCR does.
pub(crate) fn crop_quad(img: &RgbImage, quad: &[[f32; 2]; 4]) -> Option<RgbImage> {
    let [tl, tr, br, bl] = *quad;
    let w = dist(tl, tr).max(dist(bl, br));
    let h = dist(tl, bl).max(dist(tr, br));
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let axis_aligned = (tl[1] - tr[1]).abs() < 0.5
        && (bl[1] - br[1]).abs() < 0.5
        && (tl[0] - bl[0]).abs() < 0.5
        && (tr[0] - br[0]).abs() < 0.5;
    let crop = if axis_aligned {
        let x0 = tl[0].min(bl[0]).floor().max(0.0) as u32;
        let y0 = tl[1].min(tr[1]).floor().max(0.0) as u32;
        let x1 = (tr[0].max(br[0]).ceil().max(0.0) as u32).min(img.width());
        let y1 = (bl[1].max(br[1]).ceil().max(0.0) as u32).min(img.height());
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image()
    } else {
        let (ow, oh) = (
            ((w - 1e-3).ceil() as u32).clamp(1, 8192),
            ((h - 1e-3).ceil() as u32).clamp(1, 8192),
        );
        let mut out = RgbImage::new(ow, oh);
        for v in 0..oh {
            let fy = (v as f32 + 0.5) / oh as f32;
            for u in 0..ow {
                let fx = (u as f32 + 0.5) / ow as f32;
                let sx = tl[0] + fx * (tr[0] - tl[0]) + fy * (bl[0] - tl[0]) - 0.5;
                let sy = tl[1] + fx * (tr[1] - tl[1]) + fy * (bl[1] - tl[1]) - 0.5;
                out.put_pixel(u, v, bilinear(img, sx, sy));
            }
        }
        out
    };
    if crop.height() as f32 >= 1.5 * crop.width() as f32 {
        Some(imageops::rotate270(&crop))
    } else {
        Some(crop)
    }
}

fn bilinear(img: &RgbImage, x: f32, y: f32) -> Rgb<u8> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: i32, yi: i32| {
        img.get_pixel(xi.clamp(0, w - 1) as u32, yi.clamp(0, h - 1) as u32)
            .0
    };
    let (xi, yi) = (x0 as i32, y0 as i32);
    let (p00, p10, p01, p11) = (
        at(xi, yi),
        at(xi + 1, yi),
        at(xi, yi + 1),
        at(xi + 1, yi + 1),
    );
    let mut out = [0u8; 3];
    for c in 0..3 {
        let top = p00[c] as f32 * (1.0 - fx) + p10[c] as f32 * fx;
        let bot = p01[c] as f32 * (1.0 - fx) + p11[c] as f32 * fx;
        out[c] = (top * (1.0 - fy) + bot * fy + 0.5) as u8;
    }
    Rgb(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn rgba_alpha_handling() {
        let mut img = RgbaImage::new(3, 1);
        img.put_pixel(0, 0, Rgba([10, 20, 30, 0])); // GDI-style, alpha unset
        img.put_pixel(1, 0, Rgba([10, 20, 30, 255]));
        img.put_pixel(2, 0, Rgba([0, 0, 0, 128])); // half transparent black over white
        let rgb = rgba_to_rgb(&img);
        assert_eq!(rgb.get_pixel(0, 0).0, [10, 20, 30]);
        assert_eq!(rgb.get_pixel(1, 0).0, [10, 20, 30]);
        let g = rgb.get_pixel(2, 0).0[0];
        assert!((126..=129).contains(&g), "{g}");
    }

    #[test]
    fn background_is_median_of_border_not_text() {
        let mut img = RgbImage::from_pixel(40, 20, Rgb([250, 250, 250]));
        for x in 5..35 {
            for y in 6..14 {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
        assert_eq!(background_color(&img), [250, 250, 250]);
        let dark = RgbImage::from_pixel(10, 10, Rgb([30, 31, 32]));
        assert_eq!(background_color(&dark), [30, 31, 32]);
        assert_eq!(background_color(&RgbImage::new(1, 1)), [0, 0, 0]);
    }

    #[test]
    fn canvas_centres_the_image() {
        let img = RgbImage::from_pixel(10, 6, Rgb([1, 2, 3]));
        let (c, off) = centered_canvas(&img, 32, 32, [9, 9, 9]);
        assert_eq!(c.dimensions(), (32, 32));
        assert_eq!(off, (11, 13));
        assert_eq!(c.get_pixel(0, 0).0, [9, 9, 9]);
        assert_eq!(c.get_pixel(11, 13).0, [1, 2, 3]);
        assert_eq!(c.get_pixel(20, 18).0, [1, 2, 3]);
        assert_eq!(c.get_pixel(21, 19).0, [9, 9, 9]);
        assert_eq!(round_up(624, 32), 640);
        assert_eq!(round_up(640, 32), 640);
    }

    #[test]
    fn planar_bgr_normalization() {
        let mut img = RgbImage::new(2, 1);
        img.put_pixel(0, 0, Rgb([255, 0, 0])); // red
        img.put_pixel(1, 0, Rgb([0, 0, 255])); // blue
        let mut dst = vec![0.0f32; 3 * 4];
        write_bgr_planar(&mut dst, 4, 4, &img);
        // plane 0 = B, plane 1 = G, plane 2 = R; stride 4 leaves columns 2..4 untouched (0).
        assert_eq!(dst[0], -1.0);
        assert_eq!(dst[1], 1.0);
        assert_eq!(dst[4], -1.0);
        assert_eq!(dst[5], -1.0);
        assert_eq!(dst[8], 1.0);
        assert_eq!(dst[9], -1.0);
        assert_eq!(dst[2], 0.0);
    }

    #[test]
    fn crop_axis_aligned_and_clamped() {
        let mut img = RgbImage::from_pixel(50, 30, Rgb([255, 255, 255]));
        img.put_pixel(10, 10, Rgb([0, 0, 0]));
        let q = [[8.0, 8.0], [30.0, 8.0], [30.0, 20.0], [8.0, 20.0]];
        let c = crop_quad(&img, &q).unwrap();
        assert_eq!(c.dimensions(), (22, 12));
        assert_eq!(c.get_pixel(2, 2).0, [0, 0, 0]);
        // Partly outside the image: clamped, still non-empty.
        let q = [[40.0, 20.0], [60.0, 20.0], [60.0, 40.0], [40.0, 40.0]];
        assert_eq!(crop_quad(&img, &q).unwrap().dimensions(), (10, 10));
        // Degenerate.
        let q = [[5.0, 5.0], [5.2, 5.0], [5.2, 5.2], [5.0, 5.2]];
        assert!(crop_quad(&img, &q).is_none());
    }

    #[test]
    fn crop_tilted_quad_is_upright_and_vertical_text_is_rotated() {
        // A 40x10 white bar with a black left end, tilted by ~5 degrees.
        let mut img = RgbImage::from_pixel(80, 60, Rgb([255, 255, 255]));
        let (s, c) = (5f32.to_radians().sin(), 5f32.to_radians().cos());
        let origin = [10.0f32, 20.0];
        let ax = |u: f32, v: f32| [origin[0] + u * c - v * s, origin[1] + u * s + v * c];
        for v in 0..10 {
            for u in 0..40 {
                let p = ax(u as f32 + 0.5, v as f32 + 0.5);
                let col = if u < 6 { 0 } else { 255 };
                img.put_pixel(
                    p[0].floor() as u32,
                    p[1].floor() as u32,
                    Rgb([col, col, col]),
                );
            }
        }
        let q = [ax(0.0, 0.0), ax(40.0, 0.0), ax(40.0, 10.0), ax(0.0, 10.0)];
        let crop = crop_quad(&img, &q).unwrap();
        assert_eq!(crop.dimensions(), (40, 10));
        assert!(crop.get_pixel(2, 5).0[0] < 80, "left end should be dark");
        assert!(
            crop.get_pixel(30, 5).0[0] > 150,
            "right part should be light"
        );

        // Tall quad: result must be wider than tall.
        let q = [[10.0, 5.0], [20.0, 5.0], [20.0, 45.0], [10.0, 45.0]];
        let v = crop_quad(&img, &q).unwrap();
        assert_eq!(v.dimensions(), (40, 10));
    }

    /// A "text line": alternating dark/light columns over `rows`, white elsewhere.
    fn striped_line(w: u32, h: u32, x0: u32, x1: u32, rows: std::ops::Range<u32>) -> RgbImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
        for y in rows {
            for x in x0..x1 {
                if (x / 2) % 2 == 0 {
                    img.put_pixel(x, y, Rgb([0, 0, 0]));
                }
            }
        }
        img
    }

    #[test]
    fn refine_recentres_a_shifted_box() {
        let img = striped_line(120, 60, 10, 110, 20..35); // ink rows 20..=34 (15 rows)
                                                          // Detector box 4 px too low: [24, 45].
        let (y0, y1) = refine_vertical(&img, 10.0, 24.0, 110.0, 45.0);
        assert!(
            (y0 - 17.0).abs() <= 1.0 && (y1 - 38.0).abs() <= 1.0,
            "{y0} {y1}"
        );
        // Box 3 px too high and too short (ink continues 4 px below it, the search bound).
        let (y0, y1) = refine_vertical(&img, 10.0, 16.0, 110.0, 31.0);
        assert!(
            (y0 - 17.0).abs() <= 1.0 && (y1 - 38.0).abs() <= 1.0,
            "{y0} {y1}"
        );
    }

    #[test]
    fn refine_keeps_a_good_box_and_ignores_far_neighbours() {
        let mut img = striped_line(120, 90, 10, 110, 20..35);
        // A second line starting 6 rows below the first one's last ink row stays out.
        for y in 41..56 {
            for x in 10..110 {
                if (x / 2) % 2 == 0 {
                    img.put_pixel(x, y, Rgb([0, 0, 0]));
                }
            }
        }
        let (y0, y1) = refine_vertical(&img, 10.0, 17.0, 110.0, 38.0);
        assert!(
            (y0 - 17.0).abs() <= 1.0 && (y1 - 38.0).abs() <= 1.0,
            "{y0} {y1}"
        );
    }

    #[test]
    fn refine_handles_flat_gradient_and_edge_cases() {
        // No ink at all: unchanged.
        let blank = RgbImage::from_pixel(80, 40, Rgb([255, 255, 255]));
        assert_eq!(refine_vertical(&blank, 5.0, 10.0, 70.0, 30.0), (10.0, 30.0));
        // Smooth horizontal gradient only: unchanged (no strong edges).
        let mut grad = RgbImage::new(200, 40);
        for (x, _, p) in grad.enumerate_pixels_mut() {
            *p = Rgb([(x / 2) as u8, 40, 100]);
        }
        assert_eq!(refine_vertical(&grad, 5.0, 10.0, 190.0, 30.0), (10.0, 30.0));
        // Degenerate boxes do not panic.
        assert_eq!(refine_vertical(&blank, 5.0, 10.0, 6.0, 12.0), (10.0, 12.0));
        let near_edge = striped_line(120, 30, 10, 110, 0..12);
        let (y0, _) = refine_vertical(&near_edge, 10.0, 0.0, 110.0, 16.0);
        assert!(y0 >= 0.0);
    }

    #[test]
    fn resize_picks_a_filter_and_keeps_size() {
        let img = RgbImage::from_pixel(20, 10, Rgb([100, 100, 100]));
        assert_eq!(resize(&img, 40, 20).dimensions(), (40, 20));
        assert_eq!(resize(&img, 10, 5).dimensions(), (10, 5));
        assert_eq!(resize(&img, 40, 20).get_pixel(20, 10).0, [100, 100, 100]);
    }
}
