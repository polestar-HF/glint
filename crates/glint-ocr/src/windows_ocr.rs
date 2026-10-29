//! `Windows.Media.Ocr` engine (Windows 10 and later, no model files to ship).
//!
//! The `SoftwareBitmap` is built straight from BGRA bytes (`DataWriter` -> `IBuffer` ->
//! `SoftwareBitmap::CreateCopyFromBuffer`); nothing is encoded as PNG. Preprocessing follows the
//! known weak spots of the engine:
//!
//! * small crops are enlarged (3x below 40 px, 2x below 120 px) with Lanczos3, and padded with the
//!   background colour; very small regions otherwise return nothing
//! * light-on-dark images (mean luminance below 0.5) are inverted
//! * an empty first result is retried at 2x
//!
//! Recognition blocks on the WinRT async operation, so call it from a worker thread, never from
//! a UI (STA) thread.

use std::{cell::Cell, collections::HashMap, sync::Mutex};

use image::{RgbImage, RgbaImage};
use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine as WinEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
};

use crate::{
    imgproc::{background_color, centered_canvas, resize, rgba_to_rgb},
    lang::pick_language_tag,
    postprocess, OcrEngine, OcrError, OcrLine, OcrOptions, OcrResult, Result,
};

/// Padding added around the (scaled) image, in pixels.
const PAD: u32 = 10;
/// Engines for hint languages other than the default are cached, a few at most.
const MAX_CACHED: usize = 3;

fn win_err(e: windows::core::Error) -> OcrError {
    OcrError::Inference(e.to_string())
}

/// WinRT needs the thread in an apartment. Joining the MTA is harmless if the thread already has
/// one (S_FALSE) and ignored if it is an STA (`RPC_E_CHANGED_MODE`).
fn init_winrt() {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    DONE.with(|d| {
        if !d.get() {
            // SAFETY: plain COM initialization call; no pointers involved.
            let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            d.set(true);
        }
    });
}

fn engine_for_tag(tag: &str) -> Result<WinEngine> {
    let lang = Language::CreateLanguage(&tag.into()).map_err(win_err)?;
    WinEngine::TryCreateFromLanguage(&lang)
        .map_err(|e| OcrError::Load(format!("no OCR engine for {tag}: {e}")))
}

/// Windows OCR. `Send + Sync`; the WinRT engine objects are agile.
pub struct WindowsOcr {
    default: WinEngine,
    default_tag: String,
    cache: Mutex<HashMap<String, WinEngine>>,
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WindowsOcr>();
};

impl WindowsOcr {
    /// Creates an engine for `lang` (a hint such as `"en"`, `"zh-Hans"`, `"ja"`; `"auto"` or
    /// anything without an installed language pack falls back to the user profile languages).
    pub fn new(lang: &str) -> Result<Self> {
        init_winrt();
        let available = Self::available_languages().unwrap_or_default();
        let picked = pick_language_tag(lang, &available)
            .and_then(|tag| engine_for_tag(&tag).ok().map(|e| (tag, e)));
        let (default_tag, default) = match picked {
            Some(p) => p,
            None => {
                let e = WinEngine::TryCreateFromUserProfileLanguages().map_err(|e| {
                    OcrError::Load(format!(
                        "Windows OCR is unavailable: no OCR language pack matches the user profile languages ({e}). \
                         Install one under Settings > Time & language > Language & region."
                    ))
                })?;
                let tag = e
                    .RecognizerLanguage()
                    .and_then(|l| l.LanguageTag())
                    .map(|t| t.to_string())
                    .unwrap_or_default();
                (tag, e)
            }
        };
        log::info!("WindowsOcr ready, language {default_tag}");
        Ok(Self {
            default,
            default_tag,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// BCP-47 tags of the OCR languages installed on this machine (`en-US`, `zh-Hans-CN`, ...).
    pub fn available_languages() -> Result<Vec<String>> {
        init_winrt();
        let langs = WinEngine::AvailableRecognizerLanguages().map_err(win_err)?;
        let mut out = Vec::new();
        for l in &langs {
            out.push(l.LanguageTag().map_err(win_err)?.to_string());
        }
        Ok(out)
    }

    /// Language tag of the default engine.
    pub fn language(&self) -> &str {
        &self.default_tag
    }

    fn engine_for(&self, hint: &str) -> WinEngine {
        let hint = hint.trim();
        if hint.is_empty() || hint.eq_ignore_ascii_case("auto") {
            return self.default.clone();
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = cache.get(hint) {
            return e.clone();
        }
        let available = Self::available_languages().unwrap_or_default();
        let engine = pick_language_tag(hint, &available)
            .and_then(|tag| engine_for_tag(&tag).ok())
            .unwrap_or_else(|| self.default.clone());
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(hint.to_string(), engine.clone());
        engine
    }

    /// One recognition pass over `rgb` enlarged by `scale`.
    fn pass(
        &self,
        engine: &WinEngine,
        rgb: &RgbImage,
        scale: f32,
        invert: bool,
    ) -> Result<Vec<OcrLine>> {
        let (w, h) = rgb.dimensions();
        let (sw, sh) = (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        );
        let scaled;
        let src = if (sw, sh) == (w, h) {
            rgb
        } else {
            scaled = resize(rgb, sw, sh);
            &scaled
        };
        let bg = background_color(src);
        let (canvas, off) = centered_canvas(src, sw + 2 * PAD, sh + 2 * PAD, bg);
        let (cw, ch) = canvas.dimensions();

        // BGRA, opaque, inverted if the text is light on dark.
        let mut bytes = Vec::with_capacity(cw as usize * ch as usize * 4);
        for p in canvas.pixels() {
            let [r, g, b] = p.0;
            let (r, g, b) = if invert {
                (255 - r, 255 - g, 255 - b)
            } else {
                (r, g, b)
            };
            bytes.extend_from_slice(&[b, g, r, 255]);
        }
        let writer = DataWriter::new().map_err(win_err)?;
        writer.WriteBytes(&bytes).map_err(win_err)?;
        let buffer = writer.DetachBuffer().map_err(win_err)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            cw as i32,
            ch as i32,
        )
        .map_err(win_err)?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(win_err)?
            .get()
            .map_err(win_err)?;

        let mut lines = Vec::new();
        for line in &result.Lines().map_err(win_err)? {
            let text = line.Text().map_err(win_err)?.to_string();
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for word in &line.Words().map_err(win_err)? {
                let r = word.BoundingRect().map_err(win_err)?;
                x0 = x0.min(r.X);
                y0 = y0.min(r.Y);
                x1 = x1.max(r.X + r.Width);
                y1 = y1.max(r.Y + r.Height);
            }
            if x0 > x1 || text.trim().is_empty() {
                continue;
            }
            // Back to input-image pixels.
            let (x0, x1) = (
                ((x0 - off.0 as f32) / scale).clamp(0.0, w as f32),
                ((x1 - off.0 as f32) / scale).clamp(0.0, w as f32),
            );
            let (y0, y1) = (
                ((y0 - off.1 as f32) / scale).clamp(0.0, h as f32),
                ((y1 - off.1 as f32) / scale).clamp(0.0, h as f32),
            );
            // The engine reports no confidence.
            lines.push(OcrLine {
                text,
                bbox: [x0, y0, x1 - x0, y1 - y0],
                confidence: 1.0,
            });
        }
        Ok(lines)
    }
}

/// Mean luminance in `0..=1`, sampled on a grid (at most about 40k pixels).
fn mean_luminance(img: &RgbImage) -> f32 {
    let (w, h) = img.dimensions();
    let step = (((w as u64 * h as u64) / 40_000) as f64)
        .sqrt()
        .ceil()
        .max(1.0) as u32;
    let (mut sum, mut n) = (0.0f64, 0u64);
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            let p = img.get_pixel(x, y).0;
            sum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
            n += 1;
        }
    }
    if n == 0 {
        1.0
    } else {
        (sum / n as f64 / 255.0) as f32
    }
}

impl OcrEngine for WindowsOcr {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn recognize(&self, img: &RgbaImage, opts: &OcrOptions) -> Result<OcrResult> {
        init_winrt();
        let (w, h) = img.dimensions();
        let empty = || OcrResult {
            lines: Vec::new(),
            text: String::new(),
            detected_lang: None,
            engine: "windows",
        };
        if w < 2 || h < 2 {
            return Ok(empty());
        }
        let engine = self.engine_for(&opts.lang_hint);
        let rgb = rgba_to_rgb(img);
        let invert = mean_luminance(&rgb) < 0.5;

        let max_dim = WinEngine::MaxImageDimension().unwrap_or(2600) as f32;
        let fit = (max_dim - 2.0 * PAD as f32) / w.max(h) as f32;
        let wanted: f32 = match w.min(h) {
            0..=39 => 3.0,
            40..=119 => 2.0,
            _ => 1.0,
        };
        let scale = wanted.min(fit);
        let mut lines = self.pass(&engine, &rgb, scale, invert)?;
        if lines.is_empty() && scale < 2.0 && fit >= 2.0 {
            lines = self.pass(&engine, &rgb, 2.0, invert)?;
        }
        let p = postprocess::process(lines, opts.merge_lines);
        Ok(OcrResult {
            lines: p.lines,
            text: p.text,
            detected_lang: p.detected_lang,
            engine: "windows",
        })
    }
}
