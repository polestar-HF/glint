//! PP-OCR (PaddleOCR) pipeline on ONNX Runtime: DB text detection, optional angle
//! classification, CTC recognition, then the shared post-processing.
//!
//! # Model directory
//!
//! | file       | required | meaning |
//! |------------|----------|---------|
//! | `det.onnx` | yes | DB text detector (PP-OCRv6 small: `PP-OCRv6_det_small.onnx`) |
//! | `rec.onnx` | yes | CTC recognizer (`PP-OCRv6_rec_small.onnx`); RapidOCR's export embeds the character dictionary in the model metadata (key `character`) |
//! | `cls.onnx` | only with `use_angle_cls` | 0/180 degree text-line classifier |
//! | `dict.txt` | only if `rec.onnx` has no embedded dictionary | one character per line (`ppocr_keys_v1.txt` is accepted as well) |
//!
//! # Pipeline
//!
//! 1. The crop is padded (default 12 px, median border colour) and centred on a canvas whose
//!    sides are multiples of 32, so at 1x the detector sees the pixels unchanged (no resampling).
//!    Selections larger than `det_max_side` are shrunk for detection only.
//! 2. DB post-processing gives text boxes. If none are found, or all are tiny, detection is
//!    repeated at 2x (Lanczos3).
//! 3. Each box is cut from the *original-resolution* padded image and resampled once to the
//!    recognizer's height of 48 px (Lanczos3 when enlarging), so small text is upscaled with a
//!    good filter without a double resampling step.
//! 4. Lines of similar width are batched; the recognizer output is decoded greedily (CTC).
//! 5. Short low-confidence lines are dropped, then [`crate::postprocess::process`] orders,
//!    normalizes and merges the lines.

mod det;
#[cfg(any(feature = "ort-dynamic", feature = "ort-static"))]
mod engine;
#[cfg(not(any(feature = "ort-dynamic", feature = "ort-static")))]
#[path = "engine_stub.rs"]
mod engine;
mod rec;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use image::{imageops, RgbImage, RgbaImage};

use crate::{
    imgproc::{
        background_color, centered_canvas, crop_quad, refine_vertical, resize, rgba_to_rgb,
        round_up, write_bgr_planar,
    },
    postprocess, OcrEngine, OcrError, OcrLine, OcrOptions, OcrResult, Result,
};
use det::{db_boxes, DetBox, DetParams};
use rec::{ctc_decode, plan_batches, rec_width, Decoded, REC_H};

/// Box corners `[tl, tr, br, bl]` as `[x, y]`.
type Quad = [[f32; 2]; 4];

/// Tuning knobs for [`PaddleOcr`]. The defaults are what the app uses.
#[derive(Debug, Clone)]
pub struct PaddleOptions {
    /// Run the 0/180 degree classifier on every line (needs `cls.onnx`). Off by default: it
    /// costs latency and screenshots are almost never upside down.
    pub use_angle_cls: bool,
    /// ORT intra-op threads per session. `None` = `min(4, cores / 2)`, at least 1.
    pub intra_threads: Option<usize>,
    /// Location of the ONNX Runtime shared library (`ort-dynamic` builds only; ignored by
    /// `ort-static`). Checked first, before the `ORT_DYLIB_PATH` environment variable and the
    /// executable's directory. Must be a real file: it never falls back to the system search path.
    /// Only the first `PaddleOcr::load` of a process initializes ONNX Runtime; later loads reuse it.
    pub ort_dylib_path: Option<PathBuf>,
    /// Background padding around the crop in pixels (8 to 16 is sensible).
    pub pad: u32,
    /// Long side limit of the detector input. Larger selections are shrunk for detection only.
    pub det_max_side: u32,
    /// Pixel probability threshold of the DB map.
    pub det_thresh: f32,
    /// Minimum mean probability inside a box.
    pub det_box_thresh: f32,
    /// Box expansion factor (PP-OCR default 1.6).
    pub det_unclip_ratio: f32,
    /// Repeat detection at 2x when nothing, or only very small boxes, was found.
    pub retry_upscaled: bool,
    /// Maximum lines per recognizer batch (1 disables batching). Batching is compute-neutral on
    /// CPU but costs memory, so with the default area budget only short lines share a batch.
    pub rec_batch: usize,
    /// Minimum padded width of a recognizer input. PP-OCR pads to 320; 128 reads 16 short UI
    /// labels (Latin and Chinese, light and dark) just as well and halves the time spent on them.
    pub rec_min_width: usize,
    /// Widest recognizer input; longer lines are squeezed to fit.
    pub rec_max_width: usize,
    /// Upper bound on `lines * width` (pixels at height 48) per recognizer batch. The
    /// recognizer's output is `width / 8 * 18710` floats per line, so this bounds peak memory.
    pub rec_batch_max_area: usize,
    /// Lines below this confidence are dropped, but only if they are also very short.
    pub min_confidence: f32,
    /// "Very short" for the rule above, in characters.
    pub short_line_chars: usize,
    /// Probability above which the classifier flips a line by 180 degrees.
    pub cls_thresh: f32,
    /// Re-centre every upright text box vertically on its ink before recognition (helps text
    /// that the selection clips, costs well under a millisecond).
    pub snap_to_ink: bool,
}

impl Default for PaddleOptions {
    fn default() -> Self {
        Self {
            use_angle_cls: false,
            intra_threads: None,
            ort_dylib_path: None,
            pad: 12,
            det_max_side: 960,
            det_thresh: 0.3,
            det_box_thresh: 0.5,
            det_unclip_ratio: 1.6,
            retry_upscaled: true,
            rec_batch: 4,
            rec_min_width: 128,
            rec_max_width: 4800,
            rec_batch_max_area: 1280,
            min_confidence: 0.5,
            short_line_chars: 3,
            cls_thresh: 0.9,
            snap_to_ink: true,
        }
    }
}

/// Where the time of one `recognize` call went.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub prepare: Duration,
    pub detect: Duration,
    pub recognize: Duration,
    pub postprocess: Duration,
    pub total: Duration,
    /// Text boxes found by the detector.
    pub boxes: usize,
}

/// The PP-OCR engine. `Send + Sync`: the sessions are behind mutexes, so concurrent calls
/// serialize on the model.
pub struct PaddleOcr {
    engine: engine::Engine,
    opts: PaddleOptions,
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PaddleOcr>();
};

impl PaddleOcr {
    /// Loads `det.onnx`, `rec.onnx` (and `cls.onnx` if `opts.use_angle_cls`) from `model_dir`.
    /// See the [module docs](self) for the directory layout.
    pub fn load(model_dir: &Path, opts: PaddleOptions) -> Result<Self> {
        let engine = engine::Engine::load(model_dir, &opts)?;
        log::info!(
            "PaddleOcr loaded from {} ({} characters, {} threads)",
            model_dir.display(),
            engine.dict.classes() - 2,
            opts.intra_threads.unwrap_or_else(engine::default_threads)
        );
        Ok(Self { engine, opts })
    }

    /// Like [`OcrEngine::recognize`] but also reports where the time went.
    pub fn recognize_timed(
        &self,
        img: &RgbaImage,
        opts: &OcrOptions,
    ) -> Result<(OcrResult, Timings)> {
        let t0 = Instant::now();
        let mut tm = Timings::default();
        let empty = || OcrResult {
            lines: Vec::new(),
            text: String::new(),
            detected_lang: None,
            engine: "paddle",
        };
        let (w, h) = img.dimensions();
        if w < 3 || h < 3 {
            return Ok((empty(), tm));
        }

        // Pad onto a canvas with 32-multiple sides.
        let rgb = rgba_to_rgb(img);
        let bg = background_color(&rgb);
        let pad = self.opts.pad;
        let (page, off) = centered_canvas(
            &rgb,
            round_up(w + 2 * pad, 32),
            round_up(h + 2 * pad, 32),
            bg,
        );
        tm.prepare = t0.elapsed();

        // Detect.
        let t = Instant::now();
        let mut boxes = self.detect(&page, bg, 1.0)?;
        if self.opts.retry_upscaled
            && (boxes.is_empty() || median_height(&boxes) < 12.0)
            && det_factor(&page, 2.0, self.opts.det_max_side) >= 1.5
        {
            let up = self.detect(&page, bg, 2.0)?;
            if up.len() > boxes.len() {
                log::debug!(
                    "2x detection found {} boxes instead of {}",
                    up.len(),
                    boxes.len()
                );
                boxes = up;
            }
        }
        tm.detect = t.elapsed();
        tm.boxes = boxes.len();
        if boxes.is_empty() {
            tm.total = t0.elapsed();
            return Ok((empty(), tm));
        }

        // Recognize.
        let t = Instant::now();
        let bounds = [
            off.0 as f32,
            off.1 as f32,
            (off.0 + w) as f32,
            (off.1 + h) as f32,
        ];
        let (quads, decoded) = self.recognize_boxes(&page, &boxes, bounds)?;
        tm.recognize = t.elapsed();

        // Lines in input-image coordinates.
        let t = Instant::now();
        let mut lines = Vec::with_capacity(quads.len());
        for (q, d) in quads.iter().zip(decoded) {
            let text = d.text.trim().to_string();
            if text.is_empty()
                || (d.conf < self.opts.min_confidence
                    && text.chars().count() <= self.opts.short_line_chars)
            {
                continue;
            }
            let x0 = q.iter().map(|p| p[0]).fold(f32::MAX, f32::min) - off.0 as f32;
            let x1 = q.iter().map(|p| p[0]).fold(f32::MIN, f32::max) - off.0 as f32;
            let y0 = q.iter().map(|p| p[1]).fold(f32::MAX, f32::min) - off.1 as f32;
            let y1 = q.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - off.1 as f32;
            let (x0, x1) = (x0.clamp(0.0, w as f32), x1.clamp(0.0, w as f32));
            let (y0, y1) = (y0.clamp(0.0, h as f32), y1.clamp(0.0, h as f32));
            lines.push(OcrLine {
                text,
                bbox: [x0, y0, x1 - x0, y1 - y0],
                confidence: d.conf,
            });
        }
        let p = postprocess::process(lines, opts.merge_lines);
        tm.postprocess = t.elapsed();
        tm.total = t0.elapsed();
        log::debug!(
            "ocr {w}x{h}: {} boxes (mean score {:.2}), {} lines; prepare {:?}, det {:?}, rec {:?}, post {:?}",
            tm.boxes,
            boxes.iter().map(|b| b.score).sum::<f32>() / boxes.len() as f32,
            p.lines.len(),
            tm.prepare,
            tm.detect,
            tm.recognize,
            tm.postprocess
        );
        Ok((
            OcrResult {
                lines: p.lines,
                text: p.text,
                detected_lang: p.detected_lang,
                engine: "paddle",
            },
            tm,
        ))
    }

    /// Runs the detector on `page` at about `scale`x (reduced if the result would exceed
    /// `det_max_side`) and returns boxes in `page` coordinates.
    fn detect(&self, page: &RgbImage, bg: [u8; 3], scale: f32) -> Result<Vec<DetBox>> {
        let (pw, ph) = page.dimensions();
        let f = det_factor(page, scale, self.opts.det_max_side);
        let (input, fx, fy);
        let det_img: &RgbImage = if (f - 1.0).abs() < 1e-3 {
            (fx, fy) = (1.0f32, 1.0f32);
            page
        } else {
            let (nw, nh) = (
                ((pw as f32 * f).round() as u32).max(32),
                ((ph as f32 * f).round() as u32).max(32),
            );
            let scaled = resize(page, nw, nh);
            let (cw, ch) = (round_up(nw, 32), round_up(nh, 32));
            let mut canvas = RgbImage::from_pixel(cw, ch, image::Rgb(bg));
            imageops::replace(&mut canvas, &scaled, 0, 0);
            (fx, fy) = (nw as f32 / pw as f32, nh as f32 / ph as f32);
            input = canvas;
            &input
        };
        let (dw, dh) = (det_img.width() as usize, det_img.height() as usize);
        let mut data = vec![0.0f32; 3 * dw * dh];
        write_bgr_planar(&mut data, dw * dh, dw, det_img);
        let prob = self.engine.run_det(data, dh, dw)?;
        let params = DetParams {
            thresh: self.opts.det_thresh,
            box_thresh: self.opts.det_box_thresh,
            unclip_ratio: self.opts.det_unclip_ratio,
            ..DetParams::default()
        };
        let (maxx, maxy) = (pw as f32 - 1.0, ph as f32 - 1.0);
        let mut boxes = db_boxes(&prob, dw, dh, &params);
        for b in &mut boxes {
            for q in &mut b.quad {
                q[0] = (q[0] / fx).clamp(0.0, maxx);
                q[1] = (q[1] / fy).clamp(0.0, maxy);
            }
        }
        boxes.retain(|b| b.width() > 3.0 && b.height() > 3.0);
        Ok(boxes)
    }

    /// Crops, (optionally) orients, batches and decodes the boxes. Returns the (refined) quads of
    /// the boxes that could be cropped, and their decoded text, in the same order.
    fn recognize_boxes(
        &self,
        page: &RgbImage,
        boxes: &[DetBox],
        bounds: [f32; 4],
    ) -> Result<(Vec<Quad>, Vec<Decoded>)> {
        let mut kept = Vec::with_capacity(boxes.len());
        let mut crops: Vec<RgbImage> = Vec::with_capacity(boxes.len());
        for b in boxes {
            let quad = self.refined(page, &b.quad, bounds);
            if let Some(c) = crop_quad(page, &quad) {
                kept.push(quad);
                crops.push(c);
            }
        }
        if self.opts.use_angle_cls && self.engine.has_cls() {
            self.orient(&mut crops)?;
        }

        let rec_imgs: Vec<RgbImage> = crops
            .iter()
            .map(|c| {
                resize(
                    c,
                    rec_width(c.width(), c.height(), self.opts.rec_max_width) as u32,
                    REC_H as u32,
                )
            })
            .collect();
        let widths: Vec<usize> = rec_imgs.iter().map(|i| i.width() as usize).collect();
        let batches = plan_batches(
            &widths,
            self.opts.rec_batch,
            self.opts.rec_min_width,
            1.25,
            self.opts.rec_batch_max_area,
        );

        let mut out: Vec<Option<Decoded>> = vec![None; rec_imgs.len()];
        for (members, bw) in batches {
            let plane = REC_H * bw;
            let mut data = vec![0.0f32; members.len() * 3 * plane];
            for (slot, &i) in members.iter().enumerate() {
                write_bgr_planar(
                    &mut data[slot * 3 * plane..(slot + 1) * 3 * plane],
                    plane,
                    bw,
                    &rec_imgs[i],
                );
            }
            let dict = &self.engine.dict;
            let decoded = self.engine.run_rec(
                data,
                members.len(),
                bw,
                |probs, n, t, c| -> Result<Vec<Decoded>> {
                    if c != dict.classes() {
                        return Err(OcrError::Inference(format!(
                            "recognizer has {c} classes, dictionary {}",
                            dict.classes()
                        )));
                    }
                    Ok((0..n)
                        .map(|k| {
                            ctc_decode(
                                &probs[k * t * c..(k + 1) * t * c],
                                t,
                                c,
                                dict,
                                bw as f32 / t as f32,
                            )
                        })
                        .collect())
                },
            )??;
            for (&i, d) in members.iter().zip(decoded) {
                out[i] = Some(d);
            }
        }
        Ok((
            kept,
            out.into_iter()
                .map(|d| {
                    d.unwrap_or(Decoded {
                        text: String::new(),
                        conf: 0.0,
                    })
                })
                .collect(),
        ))
    }

    /// Final recognition box. Upright boxes are re-centred vertically on their ink
    /// ([`refine_vertical`]) and then clipped to the original image rectangle `bounds`
    /// (`[x0, y0, x1, y1]` in page coordinates), like PaddleOCR clips boxes to the image: the
    /// padding exists only to help detection, and text that the selection clips is read best when
    /// the crop ends exactly at the cut instead of showing the cut against a margin.
    fn refined(&self, page: &RgbImage, q: &Quad, bounds: [f32; 4]) -> Quad {
        let upright =
            q[0][1] == q[1][1] && q[2][1] == q[3][1] && q[0][0] == q[3][0] && q[1][0] == q[2][0];
        if !upright {
            return *q;
        }
        let (mut y0, mut y1) = (q[0][1], q[2][1]);
        if self.opts.snap_to_ink {
            (y0, y1) = refine_vertical(page, q[0][0], y0, q[1][0], y1);
        }
        let (x0, x1) = (
            q[0][0].clamp(bounds[0], bounds[2]),
            q[1][0].clamp(bounds[0], bounds[2]),
        );
        let (y0, y1) = (
            y0.clamp(bounds[1], bounds[3]),
            y1.clamp(bounds[1], bounds[3]),
        );
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    /// Turns lines the classifier considers upside down by 180 degrees.
    fn orient(&self, crops: &mut [RgbImage]) -> Result<()> {
        for chunk in crops.chunks_mut(6) {
            let mut data = vec![0.0f32; chunk.len() * 3 * 48 * 192];
            for (k, c) in chunk.iter().enumerate() {
                let w = ((48.0 * c.width() as f32 / c.height() as f32).ceil() as u32).clamp(8, 192);
                let r = resize(c, w, 48);
                write_bgr_planar(
                    &mut data[k * 3 * 48 * 192..(k + 1) * 3 * 48 * 192],
                    48 * 192,
                    192,
                    &r,
                );
            }
            let probs = self.engine.run_cls(data, chunk.len())?;
            for (c, p) in chunk.iter_mut().zip(probs) {
                if p[1] > self.opts.cls_thresh && p[1] > p[0] {
                    *c = imageops::rotate180(c);
                }
            }
        }
        Ok(())
    }
}

/// Scale actually applied to `page` for detection: `scale`, reduced so the long side stays
/// within `max_side`.
fn det_factor(page: &RgbImage, scale: f32, max_side: u32) -> f32 {
    let long = page.width().max(page.height()) as f32;
    let f = scale.min(max_side as f32 / long);
    // Snap "no change" so the page is used as is.
    if (f - 1.0).abs() < 0.02 {
        1.0
    } else {
        f
    }
}

fn median_height(boxes: &[DetBox]) -> f32 {
    let mut hs: Vec<f32> = boxes.iter().map(DetBox::height).collect();
    hs.sort_by(|a, b| a.total_cmp(b));
    hs.get(hs.len() / 2).copied().unwrap_or(0.0)
}

impl OcrEngine for PaddleOcr {
    fn name(&self) -> &'static str {
        "paddle"
    }

    fn recognize(&self, img: &RgbaImage, opts: &OcrOptions) -> Result<OcrResult> {
        self.recognize_timed(img, opts).map(|(r, _)| r)
    }
}
