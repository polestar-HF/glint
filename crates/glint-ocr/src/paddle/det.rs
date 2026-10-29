//! DB (Differentiable Binarization) post-processing, the same algorithm as PaddleOCR's
//! `DBPostProcess` without OpenCV, pyclipper or shapely:
//!
//! 1. threshold the probability map and dilate it with the 2x2 kernel PP-OCR uses
//! 2. 8-connected components
//! 3. minimum-area rectangle of each component (convex hull + rotating calipers)
//! 4. score = mean probability inside the rectangle, drop low scores and tiny boxes
//! 5. unclip: a rectangle offset by `d = area * ratio / perimeter` is just a bigger rectangle,
//!    so no polygon clipper is needed
//!
//! Boxes that are tilted by less than about half a degree are snapped to axis-aligned boxes,
//! which makes the recognition crop a plain, sharp sub-image.

type P = [f32; 2];

#[derive(Debug, Clone, Copy)]
pub(crate) struct DetParams {
    pub thresh: f32,
    pub box_thresh: f32,
    pub unclip_ratio: f32,
    pub min_size: f32,
    pub max_candidates: usize,
}

impl Default for DetParams {
    fn default() -> Self {
        Self {
            thresh: 0.3,
            box_thresh: 0.5,
            unclip_ratio: 1.6,
            min_size: 3.0,
            max_candidates: 1000,
        }
    }
}

/// A detected text box in probability-map coordinates, corners ordered `[tl, tr, br, bl]`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DetBox {
    pub quad: [P; 4],
    pub score: f32,
}

impl DetBox {
    /// Height of the box along its short side axis (text height estimate).
    pub fn height(&self) -> f32 {
        let d = |a: P, b: P| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        d(self.quad[0], self.quad[3]).max(d(self.quad[1], self.quad[2]))
    }
    pub fn width(&self) -> f32 {
        let d = |a: P, b: P| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        d(self.quad[0], self.quad[1]).max(d(self.quad[3], self.quad[2]))
    }
}

/// Extracts text boxes from a `w` x `h` probability map.
pub(crate) fn db_boxes(prob: &[f32], w: usize, h: usize, p: &DetParams) -> Vec<DetBox> {
    assert!(prob.len() >= w * h);
    let mut mask = binarize_dilate(prob, w, h, p.thresh);
    let mut boxes = Vec::new();
    let mut seen = 0usize;
    for_each_component(&mut mask, w, h, |pixels| {
        seen += 1;
        if seen > p.max_candidates {
            return;
        }
        if let Some(b) = box_from_component(prob, w, h, pixels, p) {
            boxes.push(b);
        }
    });
    boxes
}

fn binarize_dilate(prob: &[f32], w: usize, h: usize, thresh: f32) -> Vec<u8> {
    let bin: Vec<u8> = prob[..w * h].iter().map(|&v| (v > thresh) as u8).collect();
    // cv2.dilate with a 2x2 kernel and the default anchor: dst(x, y) = OR of src(x-1..=x, y-1..=y).
    let mut out = bin.clone();
    for y in 0..h {
        for x in 0..w {
            let mut v = bin[y * w + x];
            if x > 0 {
                v |= bin[y * w + x - 1];
            }
            if y > 0 {
                v |= bin[(y - 1) * w + x];
                if x > 0 {
                    v |= bin[(y - 1) * w + x - 1];
                }
            }
            out[y * w + x] = v;
        }
    }
    out
}

/// Calls `f` with the pixel indices of every 8-connected component of `mask` (nonzero = set).
/// `mask` is consumed (visited pixels are cleared).
fn for_each_component(mask: &mut [u8], w: usize, h: usize, mut f: impl FnMut(&[u32])) {
    let mut stack: Vec<u32> = Vec::new();
    let mut pixels: Vec<u32> = Vec::new();
    for start in 0..w * h {
        if mask[start] == 0 {
            continue;
        }
        pixels.clear();
        stack.push(start as u32);
        mask[start] = 0;
        while let Some(i) = stack.pop() {
            pixels.push(i);
            let (x, y) = (i as usize % w, i as usize / w);
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(w - 1);
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(h - 1);
            for ny in y0..=y1 {
                for nx in x0..=x1 {
                    let j = ny * w + nx;
                    if mask[j] != 0 {
                        mask[j] = 0;
                        stack.push(j as u32);
                    }
                }
            }
        }
        f(&pixels);
    }
}

fn box_from_component(
    prob: &[f32],
    w: usize,
    h: usize,
    pixels: &[u32],
    p: &DetParams,
) -> Option<DetBox> {
    // Left/right extremes per row are enough to build the convex hull.
    let (mut ymin, mut ymax) = (usize::MAX, 0usize);
    for &i in pixels {
        let y = i as usize / w;
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    let rows = ymax - ymin + 1;
    let mut ext = vec![(i32::MAX, i32::MIN); rows];
    for &i in pixels {
        let (x, y) = ((i as usize % w) as i32, i as usize / w - ymin);
        ext[y].0 = ext[y].0.min(x);
        ext[y].1 = ext[y].1.max(x);
    }
    let mut pts: Vec<(i32, i32)> = Vec::with_capacity(rows * 2);
    for (r, &(lo, hi)) in ext.iter().enumerate() {
        if lo <= hi {
            pts.push((lo, (ymin + r) as i32));
            if hi != lo {
                pts.push((hi, (ymin + r) as i32));
            }
        }
    }
    let hull = convex_hull(pts);
    let (corners, rw, rh) = min_area_rect(&hull);
    if rw.min(rh) < p.min_size {
        return None;
    }
    let score = quad_mean(prob, w, h, &corners);
    if score < p.box_thresh {
        return None;
    }
    // Unclip.
    let d = rw * rh * p.unclip_ratio / (2.0 * (rw + rh));
    let (nw, nh) = (rw + 2.0 * d, rh + 2.0 * d);
    if nw.min(nh) < p.min_size + 2.0 {
        return None;
    }
    let cx = (corners[0][0] + corners[2][0]) * 0.5;
    let cy = (corners[0][1] + corners[2][1]) * 0.5;
    // Unit axes along the rectangle sides (corners are cyclic: 0-1 is the first axis).
    let axis = |a: P, b: P, len: f32| {
        if len > 1e-6 {
            [(b[0] - a[0]) / len, (b[1] - a[1]) / len]
        } else {
            [1.0, 0.0]
        }
    };
    let u = axis(corners[0], corners[1], rw);
    let v = axis(corners[1], corners[2], rh);
    let (hu, hv) = (nw * 0.5, nh * 0.5);
    let big = [
        [cx - u[0] * hu - v[0] * hv, cy - u[1] * hu - v[1] * hv],
        [cx + u[0] * hu - v[0] * hv, cy + u[1] * hu - v[1] * hv],
        [cx + u[0] * hu + v[0] * hv, cy + u[1] * hu + v[1] * hv],
        [cx - u[0] * hu + v[0] * hv, cy - u[1] * hu + v[1] * hv],
    ];
    let mut quad = order_clockwise(big);
    // Snap an almost upright rectangle to its axis-aligned bounds.
    let bw = ((quad[1][0] - quad[0][0]).powi(2) + (quad[1][1] - quad[0][1]).powi(2)).sqrt();
    let tilt = (quad[1][1] - quad[0][1])
        .abs()
        .max((quad[2][1] - quad[3][1]).abs());
    let side_tilt = (quad[3][0] - quad[0][0])
        .abs()
        .max((quad[2][0] - quad[1][0]).abs());
    if tilt <= 0.01 * bw + 0.5 && side_tilt <= 0.01 * nh.max(nw) + 0.5 {
        let x0 = quad.iter().map(|q| q[0]).fold(f32::MAX, f32::min);
        let x1 = quad.iter().map(|q| q[0]).fold(f32::MIN, f32::max);
        let y0 = quad.iter().map(|q| q[1]).fold(f32::MAX, f32::min);
        let y1 = quad.iter().map(|q| q[1]).fold(f32::MIN, f32::max);
        quad = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    }
    let maxx = (w - 1) as f32;
    let maxy = (h - 1) as f32;
    for q in &mut quad {
        q[0] = q[0].clamp(0.0, maxx);
        q[1] = q[1].clamp(0.0, maxy);
    }
    Some(DetBox { quad, score })
}

/// Andrew's monotone chain. Returns the hull counter-clockwise (in a y-up frame) without the
/// repeated first point; fewer than three points are returned as they are.
fn convex_hull(mut pts: Vec<(i32, i32)>) -> Vec<P> {
    pts.sort_unstable();
    pts.dedup();
    if pts.len() <= 2 {
        return pts.into_iter().map(|(x, y)| [x as f32, y as f32]).collect();
    }
    let cross = |o: (i32, i32), a: (i32, i32), b: (i32, i32)| -> i64 {
        (a.0 - o.0) as i64 * (b.1 - o.1) as i64 - (a.1 - o.1) as i64 * (b.0 - o.0) as i64
    };
    let mut hull: Vec<(i32, i32)> = Vec::with_capacity(pts.len());
    for &p in &pts {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in pts.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull.into_iter()
        .map(|(x, y)| [x as f32, y as f32])
        .collect()
}

/// Minimum-area enclosing rectangle by rotating calipers over the hull edges.
/// Returns the four corners in cyclic order and the side lengths `(|c0 c1|, |c1 c2|)`.
fn min_area_rect(hull: &[P]) -> ([P; 4], f32, f32) {
    if hull.is_empty() {
        return ([[0.0; 2]; 4], 0.0, 0.0);
    }
    let mut best: Option<(f32, [P; 4], f32, f32)> = None;
    let n = hull.len();
    for i in 0..n {
        let a = hull[i];
        let b = hull[(i + 1) % n];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let (vx, vy) = (-uy, ux);
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in hull {
            let pu = p[0] * ux + p[1] * uy;
            let pv = p[0] * vx + p[1] * vy;
            u0 = u0.min(pu);
            u1 = u1.max(pu);
            v0 = v0.min(pv);
            v1 = v1.max(pv);
        }
        let (w, h) = (u1 - u0, v1 - v0);
        let area = w * h;
        if best.as_ref().is_none_or(|b| area < b.0 - 1e-4) {
            let pt = |pu: f32, pv: f32| [pu * ux + pv * vx, pu * uy + pv * vy];
            best = Some((area, [pt(u0, v0), pt(u1, v0), pt(u1, v1), pt(u0, v1)], w, h));
        }
    }
    match best {
        Some((_, c, w, h)) => (c, w, h),
        None => {
            // One point (or all coincident).
            let p = hull[0];
            ([p; 4], 0.0, 0.0)
        }
    }
}

/// PaddleOCR's `order_points_clockwise`: `[tl, tr, br, bl]`.
fn order_clockwise(pts: [P; 4]) -> [P; 4] {
    let mut s = pts;
    s.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (mut left, mut right) = ([s[0], s[1]], [s[2], s[3]]);
    left.sort_by(|a, b| a[1].total_cmp(&b[1]));
    right.sort_by(|a, b| a[1].total_cmp(&b[1]));
    [left[0], right[0], right[1], left[1]]
}

/// Mean of `prob` over the pixels whose centres lie inside (or within half a pixel of) the
/// convex quad. Falls back to the plain value under the centre for degenerate quads.
fn quad_mean(prob: &[f32], w: usize, h: usize, q: &[P; 4]) -> f32 {
    let xmin = q
        .iter()
        .map(|p| p[0])
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as usize;
    let xmax = (q
        .iter()
        .map(|p| p[0])
        .fold(f32::MIN, f32::max)
        .ceil()
        .max(0.0) as usize)
        .min(w - 1);
    let ymin = q
        .iter()
        .map(|p| p[1])
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as usize;
    let ymax = (q
        .iter()
        .map(|p| p[1])
        .fold(f32::MIN, f32::max)
        .ceil()
        .max(0.0) as usize)
        .min(h - 1);
    let area2: f32 = (0..4)
        .map(|i| {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = if area2 >= 0.0 { 1.0 } else { -1.0 };
    let (mut sum, mut n) = (0.0f64, 0u32);
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            let (px, py) = (x as f32, y as f32);
            let inside = (0..4).all(|i| {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                let len = (ex * ex + ey * ey).sqrt();
                len < 1e-6 || sign * (ex * (py - a[1]) - ey * (px - a[0])) / len >= -0.5
            });
            if inside {
                sum += prob[y * w + x] as f64;
                n += 1;
            }
        }
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probability map with filled rectangles (value 0.9) and a 0.05 background.
    fn map(w: usize, h: usize, rects: &[(usize, usize, usize, usize, f32)]) -> Vec<f32> {
        let mut m = vec![0.05f32; w * h];
        for &(x0, y0, x1, y1, v) in rects {
            for y in y0..y1 {
                for x in x0..x1 {
                    m[y * w + x] = v;
                }
            }
        }
        m
    }

    #[test]
    fn single_rectangle_is_unclipped() {
        let m = map(128, 64, &[(20, 20, 100, 36, 0.9)]); // 80 x 16 core
        let b = db_boxes(&m, 128, 64, &DetParams::default());
        assert_eq!(b.len(), 1);
        let q = b[0].quad;
        // Axis aligned after snapping.
        assert_eq!(q[0][1], q[1][1]);
        assert_eq!(q[0][0], q[3][0]);
        // d = (79 * 15 * 1.6) / (2 * 94) ~ 10.1 added on every side of the 79 x 15 centre rectangle.
        assert!(q[0][0] > 6.0 && q[0][0] < 12.0, "{q:?}");
        assert!(q[1][0] > 108.0 && q[1][0] < 114.0, "{q:?}");
        assert!(
            b[0].height() > 28.0 && b[0].height() < 40.0,
            "{}",
            b[0].height()
        );
        assert!(b[0].score > 0.8, "{}", b[0].score);
    }

    #[test]
    fn two_lines_and_noise() {
        let mut rects = vec![(10, 10, 110, 24, 0.9), (10, 50, 90, 64, 0.8)];
        rects.push((120, 5, 122, 7, 0.9)); // 2x2 speck: too small
        rects.push((60, 90, 100, 100, 0.4)); // below box_thresh
        let m = map(128, 128, &rects);
        let b = db_boxes(&m, 128, 128, &DetParams::default());
        assert_eq!(b.len(), 2, "{b:?}");
    }

    #[test]
    fn low_probability_region_is_ignored() {
        let m = map(64, 64, &[(10, 10, 50, 30, 0.2)]);
        assert!(db_boxes(&m, 64, 64, &DetParams::default()).is_empty());
    }

    #[test]
    fn components_are_eight_connected() {
        let count = |rects: &[(usize, usize, usize, usize, f32)]| {
            let mut mask: Vec<u8> = map(64, 64, rects)
                .iter()
                .map(|&v| (v > 0.5) as u8)
                .collect();
            let mut n = 0;
            for_each_component(&mut mask, 64, 64, |_| n += 1);
            n
        };
        // Blocks that meet only at a corner are one component...
        assert_eq!(count(&[(10, 10, 30, 20, 0.9), (30, 20, 50, 30, 0.9)]), 1);
        // ...a one-pixel gap separates them.
        assert_eq!(count(&[(10, 10, 30, 20, 0.9), (31, 21, 50, 30, 0.9)]), 2);
    }

    #[test]
    fn tilted_bar_keeps_its_angle() {
        // A bar 90 x 12 rotated by 10 degrees.
        let (w, h) = (160usize, 120usize);
        let mut m = vec![0.05f32; w * h];
        let (s, c) = (10f32.to_radians().sin(), 10f32.to_radians().cos());
        for v in 0..12 {
            for u in 0..90 {
                let x = 20.0 + u as f32 * c - v as f32 * s;
                let y = 40.0 + u as f32 * s + v as f32 * c;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    m[(y as usize + dy) * w + x as usize + dx] = 0.9;
                }
            }
        }
        let b = db_boxes(&m, w, h, &DetParams::default());
        assert_eq!(b.len(), 1);
        let q = b[0].quad;
        let angle = (q[1][1] - q[0][1]).atan2(q[1][0] - q[0][0]).to_degrees();
        assert!((angle - 10.0).abs() < 2.0, "angle {angle}, quad {q:?}");
        assert!(b[0].width() > 100.0);
    }

    #[test]
    fn hull_and_min_rect_basics() {
        let hull = convex_hull(vec![(0, 0), (4, 0), (4, 2), (0, 2), (2, 1), (2, 0)]);
        assert_eq!(hull.len(), 4);
        let (c, w, h) = min_area_rect(&hull);
        assert!((w * h - 8.0).abs() < 1e-3, "{w} {h}");
        assert!((w.max(h) - 4.0).abs() < 1e-3);
        let _ = c;
        // Degenerate inputs do not panic.
        assert_eq!(min_area_rect(&convex_hull(vec![(3, 3)])).1, 0.0);
        let (_, w, h) = min_area_rect(&convex_hull(vec![(0, 0), (5, 0)]));
        assert!((w.max(h) - 5.0).abs() < 1e-3 && w.min(h) < 1e-3);
        assert!(min_area_rect(&[]).1 == 0.0);
    }

    #[test]
    fn order_clockwise_matches_paddle() {
        let q = order_clockwise([[10.0, 10.0], [0.0, 10.0], [0.0, 0.0], [10.0, 0.0]]);
        assert_eq!(q, [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
    }

    #[test]
    fn boxes_are_clamped_to_the_map() {
        let m = map(64, 40, &[(0, 0, 64, 12, 0.95)]);
        let b = db_boxes(&m, 64, 40, &DetParams::default());
        assert_eq!(b.len(), 1);
        for p in b[0].quad {
            assert!(p[0] >= 0.0 && p[0] <= 63.0 && p[1] >= 0.0 && p[1] <= 39.0);
        }
    }

    #[test]
    fn candidate_limit_is_respected() {
        let mut rects = Vec::new();
        for i in 0..6 {
            rects.push((4, 4 + i * 20, 60, 14 + i * 20, 0.9));
        }
        let m = map(64, 128, &rects);
        let p = DetParams {
            max_candidates: 3,
            ..Default::default()
        };
        assert_eq!(db_boxes(&m, 64, 128, &p).len(), 3);
    }
}
