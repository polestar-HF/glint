//! Platform-independent geometry, in physical pixels.
//!
//! Every rectangle here is half-open: it covers the pixels `left..right` × `top..bottom`.
//! Coordinates are either virtual-desktop pixels (which can be negative) or pixels local
//! to one monitor; the type does not care.

use crate::PhysRect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x.saturating_add(dx), self.y.saturating_add(dy))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn from_xywh(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self::new(x, y, x.saturating_add(width), y.saturating_add(height))
    }

    pub fn from_phys(r: PhysRect) -> Self {
        let w = i32::try_from(r.width).unwrap_or(i32::MAX);
        let h = i32::try_from(r.height).unwrap_or(i32::MAX);
        Self::from_xywh(r.x, r.y, w, h)
    }

    pub fn to_phys(self) -> PhysRect {
        PhysRect {
            x: self.left,
            y: self.top,
            width: self.width().unsigned_abs(),
            height: self.height().unsigned_abs(),
        }
    }

    pub fn width(self) -> i32 {
        self.right.saturating_sub(self.left).max(0)
    }

    pub fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top).max(0)
    }

    pub fn is_empty(self) -> bool {
        self.width() == 0 || self.height() == 0
    }

    pub fn contains(self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }

    /// The overlapping part of two rectangles, or `None` if they do not overlap.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let r = Rect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        (!r.is_empty()).then_some(r)
    }

    pub fn offset(self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// Grows the rectangle by `d` on every side (shrinks when `d` is negative).
    pub fn inflate(self, d: i32) -> Rect {
        Rect::new(self.left - d, self.top - d, self.right + d, self.bottom + d)
    }

    /// The rectangle whose corner pixels are `a` and `b`, both included. Dragging from a
    /// pixel to the same pixel gives 1×1, and dragging to the last column reaches the edge.
    pub fn spanning(a: Point, b: Point) -> Rect {
        Rect::new(
            a.x.min(b.x),
            a.y.min(b.y),
            a.x.max(b.x) + 1,
            a.y.max(b.y) + 1,
        )
    }

    /// Clamps `p` to the nearest pixel inside this (non-empty) rectangle.
    pub fn clamp_point(self, p: Point) -> Point {
        Point::new(
            p.x.clamp(self.left, (self.right - 1).max(self.left)),
            p.y.clamp(self.top, (self.bottom - 1).max(self.top)),
        )
    }
}

/// Converts a logical (96-DPI) length to physical pixels, never below 1.
pub fn scaled(logical: f32, scale: f32) -> i32 {
    let s = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    ((logical * s).round() as i32).max(1)
}

/// The parts of `bounds` not covered by `hole`: at most four bands (top, bottom, left,
/// right). With no hole, or a hole outside `bounds`, that is `bounds` itself.
pub fn rects_around(bounds: Rect, hole: Option<Rect>) -> Vec<Rect> {
    let Some(hole) = hole.and_then(|h| h.intersect(bounds)) else {
        return if bounds.is_empty() {
            Vec::new()
        } else {
            vec![bounds]
        };
    };
    [
        Rect::new(bounds.left, bounds.top, bounds.right, hole.top),
        Rect::new(bounds.left, hole.bottom, bounds.right, bounds.bottom),
        Rect::new(bounds.left, hole.top, hole.left, hole.bottom),
        Rect::new(hole.right, hole.top, bounds.right, hole.bottom),
    ]
    .into_iter()
    .filter(|r| !r.is_empty())
    .collect()
}

/// A border of thickness `t` drawn just outside `r`, as four non-overlapping bands.
pub fn frame_outside(r: Rect, t: i32) -> [Rect; 4] {
    [
        Rect::new(r.left - t, r.top - t, r.right + t, r.top),
        Rect::new(r.left - t, r.bottom, r.right + t, r.bottom + t),
        Rect::new(r.left - t, r.top, r.left, r.bottom),
        Rect::new(r.right, r.top, r.right + t, r.bottom),
    ]
}

/// A border of thickness `t` drawn just inside `r`, as four non-overlapping bands.
/// Bands can come out empty when `r` is thinner than `2 * t`.
pub fn frame_inside(r: Rect, t: i32) -> [Rect; 4] {
    let t = t.min(r.width()).min(r.height()).max(0);
    [
        Rect::new(r.left, r.top, r.right, r.top + t),
        Rect::new(r.left, r.bottom - t, r.right, r.bottom),
        Rect::new(r.left, r.top + t, r.left + t, r.bottom - t),
        Rect::new(r.right - t, r.top + t, r.right, r.bottom - t),
    ]
}

/// Eight `size`×`size` handles centered on the corners and edge midpoints of `r`,
/// clockwise from the top-left corner.
pub fn handle_rects(r: Rect, size: i32) -> [Rect; 8] {
    let mx = r.left + r.width() / 2;
    let my = r.top + r.height() / 2;
    let centers = [
        (r.left, r.top),
        (mx, r.top),
        (r.right, r.top),
        (r.right, my),
        (r.right, r.bottom),
        (mx, r.bottom),
        (r.left, r.bottom),
        (r.left, my),
    ];
    let half = size / 2;
    centers.map(|(cx, cy)| Rect::from_xywh(cx - half, cy - half, size, size))
}

/// Clamps a `w`×`h` box with top-left `(x, y)` into `bounds` (top-left wins if it does
/// not fit).
fn clamp_box(x: i32, y: i32, w: i32, h: i32, bounds: Rect) -> Rect {
    let x = x.min(bounds.right - w).max(bounds.left);
    let y = y.min(bounds.bottom - h).max(bounds.top);
    Rect::from_xywh(x, y, w, h)
}

/// Places a `w`×`h` chip `gap` px above the top-left corner of `anchor`, or `gap` px
/// below that corner (inside the anchor) when there is no room above. Always kept
/// inside `bounds`.
pub fn place_chip(anchor: Rect, w: i32, h: i32, gap: i32, bounds: Rect) -> Rect {
    let above = anchor.top - gap - h;
    let y = if above >= bounds.top {
        above
    } else {
        anchor.top + gap
    };
    clamp_box(anchor.left, y, w, h, bounds)
}

/// Places a `w`×`h` box `offset` px to the bottom-right of `cursor`, flipping to the
/// left and/or above when it would leave `bounds`.
pub fn place_beside_cursor(cursor: Point, w: i32, h: i32, offset: i32, bounds: Rect) -> Rect {
    let mut x = cursor.x + offset;
    if x + w > bounds.right {
        x = cursor.x - offset - w;
    }
    let mut y = cursor.y + offset;
    if y + h > bounds.bottom {
        y = cursor.y - offset - h;
    }
    clamp_box(x, y, w, h, bounds)
}

/// The source pixels for a `cells`×`cells` magnifier centered on `center`, clipped to
/// `bounds`. Returns the clipped source and the cell offset of its top-left corner
/// within the magnifier grid, or `None` if nothing is visible.
pub fn zoom_source(center: Point, cells: i32, bounds: Rect) -> Option<(Rect, Point)> {
    let half = cells / 2;
    let full = Rect::from_xywh(center.x - half, center.y - half, cells, cells);
    let src = full.intersect(bounds)?;
    Some((src, Point::new(src.left - full.left, src.top - full.top)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phys_roundtrip_with_negative_origin() {
        let p = PhysRect {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1080,
        };
        let r = Rect::from_phys(p);
        assert_eq!(r, Rect::new(-1920, -200, 0, 880));
        assert_eq!(r.to_phys(), p);
    }

    #[test]
    fn intersect_and_contains() {
        let a = Rect::new(0, 0, 100, 100);
        assert_eq!(
            a.intersect(Rect::new(50, 50, 150, 150)),
            Some(Rect::new(50, 50, 100, 100))
        );
        assert_eq!(
            a.intersect(Rect::new(100, 0, 200, 100)),
            None,
            "touching is not overlapping"
        );
        assert!(a.contains(Point::new(0, 0)));
        assert!(a.contains(Point::new(99, 99)));
        assert!(!a.contains(Point::new(100, 50)));
        assert!(!a.contains(Point::new(-1, 50)));
    }

    #[test]
    fn spanning_includes_both_corners_in_any_direction() {
        assert_eq!(
            Rect::spanning(Point::new(5, 5), Point::new(5, 5)),
            Rect::new(5, 5, 6, 6)
        );
        assert_eq!(
            Rect::spanning(Point::new(10, 20), Point::new(13, 23)),
            Rect::new(10, 20, 14, 24)
        );
        assert_eq!(
            Rect::spanning(Point::new(13, 23), Point::new(10, 20)),
            Rect::new(10, 20, 14, 24)
        );
        assert_eq!(
            Rect::spanning(Point::new(-5, 3), Point::new(2, -4)),
            Rect::new(-5, -4, 3, 4)
        );
    }

    #[test]
    fn clamp_point_stays_on_last_pixel() {
        let m = Rect::new(-1920, 0, 0, 1080);
        assert_eq!(m.clamp_point(Point::new(500, -10)), Point::new(-1, 0));
        assert_eq!(
            m.clamp_point(Point::new(-5000, 5000)),
            Point::new(-1920, 1079)
        );
        assert_eq!(m.clamp_point(Point::new(-100, 100)), Point::new(-100, 100));
    }

    #[test]
    fn scaled_rounds_and_has_floor_of_one() {
        assert_eq!(scaled(6.0, 1.0), 6);
        assert_eq!(scaled(6.0, 1.25), 8); // 7.5 rounds away from zero
        assert_eq!(scaled(6.0, 1.5), 9);
        assert_eq!(scaled(1.0, 1.25), 1);
        assert_eq!(scaled(0.2, 1.0), 1);
        assert_eq!(scaled(2.0, f32::NAN), 2);
        assert_eq!(scaled(2.0, 0.0), 2);
    }

    #[test]
    fn rects_around_covers_everything_but_the_hole() {
        let b = Rect::new(0, 0, 100, 80);
        let hole = Rect::new(10, 20, 30, 50);
        let parts = rects_around(b, Some(hole));
        assert_eq!(parts.len(), 4);
        let area: i32 = parts.iter().map(|r| r.width() * r.height()).sum();
        assert_eq!(area, 100 * 80 - 20 * 30);
        for p in &parts {
            assert!(p.intersect(hole).is_none());
            assert_eq!(p.intersect(b), Some(*p));
        }
        // Parts must not overlap each other.
        for (i, a) in parts.iter().enumerate() {
            for c in &parts[i + 1..] {
                assert!(a.intersect(*c).is_none());
            }
        }
    }

    #[test]
    fn rects_around_edge_cases() {
        let b = Rect::new(0, 0, 100, 80);
        assert_eq!(rects_around(b, None), vec![b]);
        assert_eq!(
            rects_around(b, Some(Rect::new(200, 200, 300, 300))),
            vec![b]
        );
        assert!(rects_around(b, Some(b)).is_empty());
        // A hole sticking out of the bounds is clipped first.
        let parts = rects_around(b, Some(Rect::new(-10, -10, 50, 200)));
        assert_eq!(parts, vec![Rect::new(50, 0, 100, 80)]);
    }

    #[test]
    fn frames_have_expected_bands() {
        let r = Rect::new(10, 10, 20, 30);
        let out = frame_outside(r, 2);
        let area: i32 = out.iter().map(|b| b.width() * b.height()).sum();
        assert_eq!(area, 14 * 24 - 10 * 20);
        assert!(out.iter().all(|b| b.intersect(r).is_none()));

        let inside = frame_inside(r, 2);
        let area: i32 = inside.iter().map(|b| b.width() * b.height()).sum();
        assert_eq!(area, 10 * 20 - 6 * 16);
        assert!(inside
            .iter()
            .all(|b| b.is_empty() || b.intersect(r) == Some(*b)));
    }

    #[test]
    fn frame_inside_degenerate_is_safe() {
        let thin = Rect::new(0, 0, 3, 100);
        for b in frame_inside(thin, 2) {
            assert!(b.is_empty() || thin.intersect(b) == Some(b));
        }
    }

    #[test]
    fn handles_are_centered_on_corners_and_midpoints() {
        let r = Rect::new(100, 200, 300, 400);
        let h = handle_rects(r, 6);
        assert_eq!(h[0], Rect::new(97, 197, 103, 203));
        assert_eq!(h[1], Rect::new(197, 197, 203, 203));
        assert_eq!(h[4], Rect::new(297, 397, 303, 403));
        assert_eq!(h[7], Rect::new(97, 297, 103, 303));
        assert!(h.iter().all(|x| x.width() == 6 && x.height() == 6));
    }

    #[test]
    fn chip_goes_above_then_below() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let sel = Rect::new(100, 200, 500, 400);
        assert_eq!(
            place_chip(sel, 80, 24, 4, bounds),
            Rect::new(100, 172, 180, 196)
        );
        // No room above: below the top-left corner, inside the selection.
        let top = Rect::new(100, 10, 500, 400);
        assert_eq!(
            place_chip(top, 80, 24, 4, bounds),
            Rect::new(100, 14, 180, 38)
        );
        // Right edge: shifted left to stay on screen.
        let right = Rect::new(1900, 500, 1920, 600);
        assert_eq!(
            place_chip(right, 80, 24, 4, bounds),
            Rect::new(1840, 472, 1920, 496)
        );
    }

    #[test]
    fn beside_cursor_flips_at_edges() {
        let b = Rect::new(0, 0, 1000, 800);
        assert_eq!(
            place_beside_cursor(Point::new(100, 100), 120, 150, 20, b),
            Rect::new(120, 120, 240, 270)
        );
        // Right edge flips left.
        assert_eq!(
            place_beside_cursor(Point::new(950, 100), 120, 150, 20, b),
            Rect::new(810, 120, 930, 270)
        );
        // Bottom edge flips up.
        assert_eq!(
            place_beside_cursor(Point::new(100, 750), 120, 150, 20, b),
            Rect::new(120, 580, 240, 730)
        );
        // Both.
        assert_eq!(
            place_beside_cursor(Point::new(990, 790), 120, 150, 20, b),
            Rect::new(850, 620, 970, 770)
        );
        // Too close to both sides: clamped inside.
        let tiny = Rect::new(0, 0, 130, 160);
        let r = place_beside_cursor(Point::new(65, 80), 120, 150, 20, tiny);
        assert_eq!(r.intersect(tiny), Some(r));
    }

    #[test]
    fn zoom_source_clips_at_edges() {
        let b = Rect::new(0, 0, 100, 100);
        let (src, off) = zoom_source(Point::new(50, 50), 15, b).expect("visible");
        assert_eq!(src, Rect::new(43, 43, 58, 58));
        assert_eq!(off, Point::new(0, 0));
        let (src, off) = zoom_source(Point::new(2, 99), 15, b).expect("visible");
        assert_eq!(src, Rect::new(0, 92, 10, 100));
        assert_eq!(off, Point::new(5, 0));
        assert!(zoom_source(Point::new(-50, -50), 15, b).is_none());
    }
}
