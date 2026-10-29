//! Platform-independent scene layout: turns a monitor's [`View`] into a flat list of
//! draw primitives in monitor-local physical pixels. The renderer only executes them,
//! so the whole visual spec (sizes, placement, flipping) is unit-tested here.

use super::input::{Highlight, View};
use crate::geom::{self, Point, Rect};
use crate::pixels;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgb(rgb: u32, a: f32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
            g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
            b: (rgb & 0xFF) as f32 / 255.0,
            a,
        }
    }

    fn from_bytes(c: [u8; 3]) -> Self {
        Self::rgb(u32::from_be_bytes([0, c[0], c[1], c[2]]), 1.0)
    }

    fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }
}

pub const DIM: Color = Color::rgb(0x000000, 0.45);
pub const CHIP_BG: Color = Color::rgb(0x1F1F1F, 0.90);
pub const CHIP_FG: Color = Color::rgb(0xF2F2F2, 1.0);
const WHITE: Color = Color::rgb(0xFFFFFF, 1.0);
const BLACK: Color = Color::rgb(0x000000, 1.0);
const GRID: Color = Color::rgb(0x808080, 0.35);
const CROSSHAIR_ALPHA: f32 = 0.35;

/// Text size of the chips, in logical px.
pub const FONT_SIZE: f32 = 12.0;
/// Magnified pixels per side (odd, so one pixel sits in the middle).
pub const MAG_CELLS: i32 = 15;
/// Logical px per magnified pixel: 15 × 8 = 120 logical px box.
const MAG_ZOOM: f32 = 8.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    /// The whole frozen frame, 1:1.
    Frozen,
    /// Aliased (pixel-crisp) solid fill.
    Fill { rect: Rect, color: Color },
    /// Antialiased rounded fill.
    RoundFill { rect: Rect, radius: f32, color: Color },
    /// Nearest-neighbour magnification of `src` (frame pixels) into `dst`.
    Zoom { src: Rect, dst: Rect },
    /// One line of text centered in `rect`.
    Text { rect: Rect, text: String, color: Color },
}

pub struct Style {
    /// Monitor DPI scale (1.0 = 96 DPI).
    pub scale: f32,
    /// 0xRRGGBB.
    pub accent: u32,
}

/// Builds the scene for one monitor of `size` px. `view` is monitor-local, `sample` is
/// the color under the magnifier, and `measure` returns a text line's (width, height) in
/// physical px for the overlay font.
pub fn build(
    view: &View,
    size: (i32, i32),
    style: &Style,
    sample: Option<[u8; 3]>,
    measure: &mut dyn FnMut(&str) -> (f32, f32),
) -> Vec<Prim> {
    let mut s = Scene {
        out: Vec::with_capacity(72),
        bounds: Rect::new(0, 0, size.0, size.1),
        scale: style.scale,
        accent: Color::rgb(style.accent, 1.0),
    };
    s.out.push(Prim::Frozen);

    let hole = view.highlight.rect().and_then(|r| r.intersect(s.bounds));
    for r in geom::rects_around(s.bounds, hole) {
        s.fill(r, DIM);
    }
    if let Some(h) = hole {
        let border = match view.highlight {
            Highlight::Selection(_) => {
                let t = s.px(1.0);
                for b in geom::frame_outside(h, t) {
                    s.fill(b, s.accent);
                }
                let (size, edge) = (s.px(6.0), s.px(1.0));
                for r in geom::handle_rects(h, size) {
                    s.fill(r, s.accent);
                    s.fill(r.inflate(-edge), WHITE);
                }
                t
            }
            _ => {
                for b in geom::frame_inside(h, s.px(2.0)) {
                    s.fill(b, s.accent);
                }
                0
            }
        };
        s.size_chip(h, border, measure);
    }
    if let Some(c) = view.magnifier {
        if s.bounds.contains(c) {
            s.magnifier(c, sample.unwrap_or([0, 0, 0]), measure);
        }
    }
    s.out
}

struct Scene {
    out: Vec<Prim>,
    bounds: Rect,
    scale: f32,
    accent: Color,
}

impl Scene {
    fn px(&self, logical: f32) -> i32 {
        geom::scaled(logical, self.scale)
    }

    fn fill(&mut self, rect: Rect, color: Color) {
        if let Some(rect) = rect.intersect(self.bounds) {
            self.out.push(Prim::Fill { rect, color });
        }
    }

    fn radius(&self) -> f32 {
        4.0 * self.scale
    }

    /// Chip size for one line of text with the spec's 4×8 logical px padding.
    fn chip_size(&self, text_w: f32, text_h: f32) -> (i32, i32) {
        (text_w.ceil() as i32 + 2 * self.px(8.0), text_h.ceil() as i32 + 2 * self.px(4.0))
    }

    fn size_chip(&mut self, sel: Rect, border: i32, measure: &mut dyn FnMut(&str) -> (f32, f32)) {
        let text = format!("{} \u{00D7} {}", sel.width(), sel.height());
        let (tw, th) = measure(&text);
        let (w, h) = self.chip_size(tw, th);
        let chip = geom::place_chip(sel, w, h, border + self.px(4.0), self.bounds);
        self.out.push(Prim::RoundFill { rect: chip, radius: self.radius(), color: CHIP_BG });
        self.out.push(Prim::Text { rect: chip, text, color: CHIP_FG });
    }

    fn magnifier(&mut self, c: Point, rgb: [u8; 3], measure: &mut dyn FnMut(&str) -> (f32, f32)) {
        let cell = self.px(MAG_ZOOM);
        let side = MAG_CELLS * cell;
        let outline = self.px(1.0);
        let gap = self.px(4.0);
        let hex = pixels::hex(rgb);
        let (tw, th) = measure(&hex);
        let (_, bar_h) = self.chip_size(tw, th);
        let group_w = side + 2 * outline;
        let framed_h = side + 2 * outline;
        let group = geom::place_beside_cursor(c, group_w, framed_h + gap + bar_h, self.px(20.0), self.bounds);

        // Box: dark outline, black where the source runs off the monitor, then pixels.
        let framed = Rect::from_xywh(group.left, group.top, group_w, framed_h);
        let zoom_box = framed.inflate(-outline);
        self.fill(framed, CHIP_BG);
        self.fill(zoom_box, BLACK);
        if let Some((src, off)) = geom::zoom_source(c, MAG_CELLS, self.bounds) {
            let dst = Rect::from_xywh(
                zoom_box.left + off.x * cell,
                zoom_box.top + off.y * cell,
                src.width() * cell,
                src.height() * cell,
            );
            self.out.push(Prim::Zoom { src, dst });
        }

        // Pixel grid: 1 physical px lines between magnified pixels.
        for i in 1..MAG_CELLS {
            let x = zoom_box.left + i * cell;
            let y = zoom_box.top + i * cell;
            self.fill(Rect::new(x, zoom_box.top, x + 1, zoom_box.bottom), GRID);
            self.fill(Rect::new(zoom_box.left, y, zoom_box.right, y + 1), GRID);
        }

        // Crosshair: tinted center row and column, and an outlined center pixel whose
        // outline contrasts with the pixel itself.
        let mid = MAG_CELLS / 2;
        let center = Rect::from_xywh(zoom_box.left + mid * cell, zoom_box.top + mid * cell, cell, cell);
        let tint = self.accent.with_alpha(CROSSHAIR_ALPHA);
        self.fill(Rect::new(zoom_box.left, center.top, center.left, center.bottom), tint);
        self.fill(Rect::new(center.right, center.top, zoom_box.right, center.bottom), tint);
        self.fill(Rect::new(center.left, zoom_box.top, center.right, center.top), tint);
        self.fill(Rect::new(center.left, center.bottom, center.right, zoom_box.bottom), tint);
        let contrast = if pixels::luminance(rgb) > 0.5 { BLACK } else { WHITE };
        for b in geom::frame_inside(center, self.px(1.0)) {
            self.fill(b, contrast);
        }

        // Hex line: swatch + "#RRGGBB", centered in a chip as wide as the box.
        let bar = Rect::from_xywh(group.left, framed.bottom + gap, group_w, bar_h);
        self.out.push(Prim::RoundFill { rect: bar, radius: self.radius(), color: CHIP_BG });
        let swatch_side = self.px(10.0);
        let swatch_gap = self.px(6.0);
        let text_w = tw.ceil() as i32;
        let content = swatch_side + swatch_gap + text_w;
        let x0 = bar.left + (bar.width() - content) / 2;
        let swatch = Rect::from_xywh(x0, bar.top + (bar.height() - swatch_side) / 2, swatch_side, swatch_side);
        self.fill(swatch, CHIP_FG.with_alpha(0.8));
        self.fill(swatch.inflate(-self.px(1.0)), Color::from_bytes(rgb));
        let text_left = swatch.right + swatch_gap;
        let text_rect = Rect::new(text_left, bar.top, text_left + text_w, bar.bottom);
        self.out.push(Prim::Text { rect: text_rect, text: hex, color: CHIP_FG });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (i32, i32) = (1920, 1080);

    fn measure(s: &str) -> (f32, f32) {
        (s.chars().count() as f32 * 7.0, 16.0)
    }

    fn build_at(view: View, scale: f32) -> Vec<Prim> {
        build(&view, SIZE, &Style { scale, accent: 0x2F6FEB }, Some([0x12, 0x34, 0x56]), &mut measure)
    }

    fn fills_with(prims: &[Prim], color: Color) -> Vec<Rect> {
        prims
            .iter()
            .filter_map(|p| match p {
                Prim::Fill { rect, color: c } if *c == color => Some(*rect),
                _ => None,
            })
            .collect()
    }

    fn texts(prims: &[Prim]) -> Vec<(Rect, String)> {
        prims
            .iter()
            .filter_map(|p| match p {
                Prim::Text { rect, text, .. } => Some((*rect, text.clone())),
                _ => None,
            })
            .collect()
    }

    fn assert_in_bounds(prims: &[Prim]) {
        let b = Rect::new(0, 0, SIZE.0, SIZE.1);
        for p in prims {
            let r = match p {
                Prim::Frozen => continue,
                Prim::Fill { rect, .. } | Prim::RoundFill { rect, .. } | Prim::Text { rect, .. } => *rect,
                Prim::Zoom { dst, .. } => *dst,
            };
            assert_eq!(r.intersect(b), Some(r), "{p:?} leaves the monitor");
        }
    }

    #[test]
    fn idle_without_cursor_is_frozen_frame_fully_dimmed() {
        let prims = build_at(View::default(), 1.0);
        assert_eq!(prims, vec![Prim::Frozen, Prim::Fill { rect: Rect::new(0, 0, 1920, 1080), color: DIM }]);
    }

    #[test]
    fn selection_has_undimmed_hole_border_handles_and_chip() {
        let sel = Rect::new(400, 300, 600, 400);
        let prims = build_at(View { highlight: Highlight::Selection(sel), magnifier: None }, 1.0);
        assert_eq!(prims[0], Prim::Frozen);

        let dim = fills_with(&prims, DIM);
        assert_eq!(dim.len(), 4);
        assert!(dim.iter().all(|r| r.intersect(sel).is_none()));
        let dim_area: i32 = dim.iter().map(|r| r.width() * r.height()).sum();
        assert_eq!(dim_area, 1920 * 1080 - 200 * 100);

        let accent = Color::rgb(0x2F6FEB, 1.0);
        let accent_fills = fills_with(&prims, accent);
        assert_eq!(accent_fills.len(), 4 + 8, "border bands + handle outlines");
        assert!(accent_fills[..4].iter().all(|b| b.width() == 1 || b.height() == 1));
        let handles = fills_with(&prims, WHITE);
        assert_eq!(handles.len(), 8);
        assert!(handles.iter().all(|h| h.width() == 4 && h.height() == 4), "6 px handle minus 1 px edge");

        let t = texts(&prims);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1, "200 × 100");
        // 9 chars × 7 = 63 + 16 padding = 79 wide, 16 + 8 = 24 tall, 5 px above the top.
        assert_eq!(t[0].0, Rect::new(400, 271, 479, 295));
        assert_in_bounds(&prims);
    }

    #[test]
    fn snap_uses_inside_border_and_chip_below_when_at_top() {
        let win = Rect::new(0, 0, 1920, 1040);
        let prims = build_at(View { highlight: Highlight::Snap(win), magnifier: None }, 1.0);
        let accent = fills_with(&prims, Color::rgb(0x2F6FEB, 1.0));
        assert_eq!(accent.len(), 4);
        assert!(accent.iter().all(|b| win.intersect(*b) == Some(*b)), "snap outline is inside");
        assert_eq!(accent[0].height(), 2);
        assert!(fills_with(&prims, WHITE).is_empty(), "no handles while snapping");
        let t = texts(&prims);
        assert_eq!(t[0].0.top, 4, "chip below the top-left corner");
        assert_in_bounds(&prims);
    }

    #[test]
    fn logical_sizes_scale_with_dpi() {
        let sel = Rect::new(400, 300, 600, 400);
        let prims = build_at(View { highlight: Highlight::Selection(sel), magnifier: None }, 1.5);
        let accent = fills_with(&prims, Color::rgb(0x2F6FEB, 1.0));
        assert_eq!(accent[0].height(), 2, "1 px × 1.5 rounds to 2");
        assert_eq!(accent[4].width(), 9, "6 px handle × 1.5");
        let t = texts(&prims);
        assert_eq!(t[0].0.height(), 16 + 2 * 6);
    }

    #[test]
    fn magnifier_layout_and_flip() {
        let c = Point::new(100, 100);
        let prims = build_at(View { highlight: Highlight::None, magnifier: Some(c) }, 1.0);
        let zoom = prims.iter().find_map(|p| match p {
            Prim::Zoom { src, dst } => Some((*src, *dst)),
            _ => None,
        });
        let (src, dst) = zoom.expect("zoom prim");
        assert_eq!(src, Rect::new(93, 93, 108, 108));
        assert_eq!(dst.width(), 120);
        assert_eq!(dst.height(), 120);
        assert!(dst.left > c.x && dst.top > c.y, "bottom-right of the cursor");
        let t = texts(&prims);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1, "#123456");
        assert!(t[0].0.top > dst.bottom, "hex line under the box");

        let corner = Point::new(1910, 1070);
        let prims = build_at(View { highlight: Highlight::None, magnifier: Some(corner) }, 2.0);
        let (_, dst) = prims
            .iter()
            .find_map(|p| match p {
                Prim::Zoom { src, dst } => Some((*src, *dst)),
                _ => None,
            })
            .expect("zoom prim");
        assert!(dst.right <= corner.x && dst.bottom <= corner.y, "flipped to the top-left");
        assert_in_bounds(&prims);
    }

    #[test]
    fn magnifier_at_monitor_edge_keeps_cells_aligned() {
        let prims = build_at(View { highlight: Highlight::None, magnifier: Some(Point::new(0, 0)) }, 1.0);
        let (src, dst) = prims
            .iter()
            .find_map(|p| match p {
                Prim::Zoom { src, dst } => Some((*src, *dst)),
                _ => None,
            })
            .expect("zoom prim");
        assert_eq!(src, Rect::new(0, 0, 8, 8));
        assert_eq!((dst.width(), dst.height()), (64, 64));
        // The source's top-left pixel lands in grid cell (7, 7), i.e. the center.
        let black = fills_with(&prims, BLACK);
        let zoom_box = black[0];
        assert_eq!(dst.left - zoom_box.left, 7 * 8);
        assert_eq!(dst.top - zoom_box.top, 7 * 8);
        assert_in_bounds(&prims);
    }

    #[test]
    fn center_outline_contrasts_with_pixel() {
        let view = View { highlight: Highlight::None, magnifier: Some(Point::new(500, 500)) };
        let bright = build(&view, SIZE, &Style { scale: 1.0, accent: 0x2F6FEB }, Some([250, 250, 250]), &mut measure);
        let dark = build(&view, SIZE, &Style { scale: 1.0, accent: 0x2F6FEB }, Some([5, 5, 5]), &mut measure);
        // BLACK is also the zoom box backdrop, so compare counts.
        assert!(fills_with(&bright, BLACK).len() > fills_with(&dark, BLACK).len());
        assert!(fills_with(&dark, WHITE).len() >= 4);
    }
}
