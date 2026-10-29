//! Direct2D + DirectWrite execution of a layout scene into one overlay window.
//!
//! Render targets are created at 96 DPI so one DIP is one physical pixel; all DPI
//! scaling is done by the layout (`Style::scale`), fonts included.

use windows::core::{w, Result as WinResult, BOOL, PCWSTR};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_DEBUG_LEVEL_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_OPTIONS,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use super::input::View;
use super::layout::{self, Color, Prim, Style};
use crate::geom::Rect;
use crate::{pixels, MonitorFrame};

/// Device-independent factories, shared by all windows of one overlay session.
pub struct Gfx {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    variable_font: bool,
}

impl Gfx {
    pub fn new() -> WinResult<Self> {
        let options = D2D1_FACTORY_OPTIONS { debugLevel: D2D1_DEBUG_LEVEL_NONE };
        // SAFETY: factory constructors with valid arguments; `options` outlives the call.
        let d2d = unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options)) }?;
        // SAFETY: as above.
        let dwrite = unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) }?;
        let variable_font = has_family(&dwrite, w!("Segoe UI Variable Text"));
        Ok(Self { d2d, dwrite, variable_font })
    }

    fn family(&self) -> PCWSTR {
        if self.variable_font {
            w!("Segoe UI Variable Text")
        } else {
            w!("Segoe UI")
        }
    }
}

fn has_family(dwrite: &IDWriteFactory, name: PCWSTR) -> bool {
    let mut collection = None;
    // SAFETY: out-pointers are valid locals; `name` is a static NUL-terminated literal.
    unsafe {
        if dwrite.GetSystemFontCollection(&mut collection, false).is_err() {
            return false;
        }
        let Some(collection) = collection else { return false };
        let (mut index, mut exists) = (0u32, BOOL(0));
        collection.FindFamilyName(name, &mut index, &mut exists).is_ok() && exists.as_bool()
    }
}

/// Result of one draw call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawn {
    Presented,
    /// The GPU device was lost (`D2DERR_RECREATE_TARGET`). Device resources were
    /// released; invalidate the window so the next paint recreates them.
    DeviceLost,
}

/// Per-window renderer: the DPI-scaled text format plus lazily created device resources.
pub struct Renderer {
    hwnd: HWND,
    style: Style,
    font_px: f32,
    format: IDWriteTextFormat,
    device: Option<Device>,
}

/// Device-dependent resources. Dropping them releases the COM objects.
struct Device {
    target: ID2D1HwndRenderTarget,
    frozen: ID2D1Bitmap,
    brush: ID2D1SolidColorBrush,
}

impl Renderer {
    pub fn new(gfx: &Gfx, hwnd: HWND, scale: f32, accent: u32) -> WinResult<Self> {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let font_px = layout::FONT_SIZE * scale;
        // SAFETY: valid arguments; the family and locale are static NUL-terminated literals.
        let format = unsafe {
            gfx.dwrite.CreateTextFormat(
                gfx.family(),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font_px,
                w!("en-us"),
            )
        }?;
        // SAFETY: plain setters on a live object.
        unsafe {
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }
        Ok(Self { hwnd, style: Style { scale, accent }, font_px, format, device: None })
    }

    /// Draws `view` (monitor-local) over the frozen `frame` and presents it.
    pub fn draw(&mut self, gfx: &Gfx, frame: &MonitorFrame, view: &View) -> WinResult<Drawn> {
        if self.device.is_none() {
            self.device = Some(Device::create(gfx, self.hwnd, frame)?);
        }
        let Some(dev) = self.device.as_ref() else {
            return Ok(Drawn::DeviceLost);
        };

        let (w, h) = (frame.rect.width, frame.rect.height);
        let sample = view.magnifier.and_then(|p| pixels::sample_rgb(&frame.bgra, w, h, p));
        let (dwrite, format, font_px) = (&gfx.dwrite, &self.format, self.font_px);
        let mut measure = |text: &str| measure_text(dwrite, format, font_px, text);
        let size = (w as i32, h as i32);
        let prims = layout::build(view, size, &self.style, sample, &mut measure);

        let full = rect_f(Rect::new(0, 0, size.0, size.1));
        // SAFETY: BeginDraw/EndDraw are paired below with no early return in between;
        // every resource used belongs to this render target.
        let end = unsafe {
            dev.target.BeginDraw();
            dev.target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            dev.target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dev.target.Clear(Some(&color_f(Color::rgb(0, 1.0))));
            for p in &prims {
                dev.execute(p, format, &full);
            }
            dev.target.EndDraw(None, None)
        };
        match end {
            Ok(()) => Ok(Drawn::Presented),
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                log::warn!("overlay render target lost; recreating");
                self.device = None;
                Ok(Drawn::DeviceLost)
            }
            Err(e) => Err(e),
        }
    }
}

impl Device {
    fn create(gfx: &Gfx, hwnd: HWND, frame: &MonitorFrame) -> WinResult<Self> {
        let (w, h) = (frame.rect.width, frame.rect.height);
        let mut target = create_target(gfx, hwnd, w, h, D2D1_RENDER_TARGET_TYPE_DEFAULT)?;
        // SAFETY: plain getter on a live object.
        let max = unsafe { target.GetMaximumBitmapSize() };
        if w.max(h) > max {
            // Old GPUs (feature level 9.x) cap textures at 2048/4096 px; the software
            // rasterizer has no such limit.
            log::warn!("monitor {w}x{h} exceeds GPU bitmap limit {max}; using software rendering");
            target = create_target(gfx, hwnd, w, h, D2D1_RENDER_TARGET_TYPE_SOFTWARE)?;
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        // SAFETY: select_region checked `frame.bgra.len() == w * h * 4`, so the pointer
        // covers `h` rows of `pitch = w * 4` bytes. D2D copies the pixels before returning.
        let frozen = unsafe {
            target.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(frame.bgra.as_ptr().cast()), w * 4, &props)
        }?;
        // SAFETY: valid color pointer for the duration of the call.
        let brush = unsafe { target.CreateSolidColorBrush(&color_f(layout::CHIP_FG), None) }?;
        Ok(Self { target, frozen, brush })
    }

    /// Executes one primitive. Must be called between BeginDraw and EndDraw.
    unsafe fn execute(&self, prim: &Prim, format: &IDWriteTextFormat, full: &D2D_RECT_F) {
        let t = &self.target;
        // SAFETY: the caller is inside BeginDraw/EndDraw; all pointers are to locals that
        // outlive each call, and all resources belong to `t`.
        unsafe {
            match prim {
                Prim::Frozen => {
                    t.DrawBitmap(&self.frozen, Some(full), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
                }
                Prim::Fill { rect, color } => {
                    self.brush.SetColor(&color_f(*color));
                    t.FillRectangle(&rect_f(*rect), &self.brush);
                }
                Prim::RoundFill { rect, radius, color } => {
                    self.brush.SetColor(&color_f(*color));
                    t.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    let rr = D2D1_ROUNDED_RECT { rect: rect_f(*rect), radiusX: *radius, radiusY: *radius };
                    t.FillRoundedRectangle(&rr, &self.brush);
                    t.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                }
                Prim::Zoom { src, dst } => {
                    t.DrawBitmap(
                        &self.frozen,
                        Some(&rect_f(*dst)),
                        1.0,
                        D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                        Some(&rect_f(*src)),
                    );
                }
                Prim::Text { rect, text, color } => {
                    let wide: Vec<u16> = text.encode_utf16().collect();
                    self.brush.SetColor(&color_f(*color));
                    t.DrawText(
                        &wide,
                        format,
                        &rect_f(*rect),
                        &self.brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                }
            }
        }
    }
}

fn create_target(
    gfx: &Gfx,
    hwnd: HWND,
    w: u32,
    h: u32,
    kind: D2D1_RENDER_TARGET_TYPE,
) -> WinResult<ID2D1HwndRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: kind,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        // 96 DPI: one DIP is one physical pixel.
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: D2D_SIZE_U { width: w, height: h },
        // Repaints are driven by input and coalesced by WM_PAINT; don't block on vblank.
        presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    };
    // SAFETY: `hwnd` is a live window owned by this thread; props outlive the call.
    unsafe { gfx.d2d.CreateHwndRenderTarget(&props, &hwnd_props) }
}

fn measure_text(dwrite: &IDWriteFactory, format: &IDWriteTextFormat, font_px: f32, text: &str) -> (f32, f32) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: `wide` and `metrics` are locals that outlive the calls.
    let metrics = unsafe {
        dwrite.CreateTextLayout(&wide, format, 4096.0, 4096.0).and_then(|layout| {
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m).map(|()| m)
        })
    };
    match metrics {
        Ok(m) => (m.width, m.height),
        Err(e) => {
            log::debug!("text measurement failed: {e}");
            (font_px * 0.6 * text.chars().count() as f32, font_px * 1.33)
        }
    }
}

fn rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.left as f32, top: r.top as f32, right: r.right as f32, bottom: r.bottom as f32 }
}

fn color_f(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}
