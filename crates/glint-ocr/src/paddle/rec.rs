//! Text recognition helpers: character dictionary, CTC decoding, batching and width planning.
//!
//! The PP-OCRv6 recognizer takes `[N, 3, 48, W]` (BGR, `(x/255 - 0.5) / 0.5`) with a dynamic `W`
//! and returns per-timestep class probabilities `[N, T, C]`. Class 0 is the CTC blank, classes
//! `1..=n` are the dictionary entries and class `n + 1` is the space.

use crate::{OcrError, Result};

/// Input height of the recognizer.
pub(crate) const REC_H: usize = 48;

#[derive(Debug, Clone)]
pub(crate) struct Dict {
    chars: Vec<Box<str>>,
}

#[cfg_attr(
    not(any(feature = "ort-dynamic", feature = "ort-static")),
    allow(dead_code)
)]
impl Dict {
    /// From the model's `character` metadata (entries separated by `\n`, no trailing newline).
    pub fn from_metadata(s: &str) -> Result<Self> {
        let chars: Vec<Box<str>> = s.split('\n').map(Box::from).collect();
        Self::checked(chars)
    }

    /// From a `ppocr_keys`-style file: one entry per line.
    pub fn from_file_text(s: &str) -> Result<Self> {
        let s = s.strip_prefix('\u{feff}').unwrap_or(s);
        let mut chars: Vec<Box<str>> = s
            .split('\n')
            .map(|l| Box::from(l.strip_suffix('\r').unwrap_or(l)))
            .collect();
        // A final newline does not add an entry.
        if chars.last().is_some_and(|l| l.is_empty()) {
            chars.pop();
        }
        Self::checked(chars)
    }

    fn checked(chars: Vec<Box<str>>) -> Result<Self> {
        if chars.is_empty() || chars.iter().all(|c| c.is_empty()) {
            return Err(OcrError::Load("character dictionary is empty".into()));
        }
        Ok(Self { chars })
    }

    /// Number of classes the model must output: blank + entries + space.
    pub fn classes(&self) -> usize {
        self.chars.len() + 2
    }

    fn get(&self, class: usize) -> &str {
        match class {
            0 => "",
            c if c <= self.chars.len() => &self.chars[c - 1],
            c if c == self.chars.len() + 1 => " ",
            _ => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decoded {
    pub text: String,
    /// Mean probability of the emitted characters (0 when nothing was emitted).
    pub conf: f32,
}

/// A wide empty gap is a word break even if the model did not emit a space: the gap between two
/// emitted characters must exceed this many median character pitches...
const WORD_GAP_PITCHES: f32 = 2.2;
/// ...and this many pixels (at the recognizer's height of 48). Measured on clean text, adjacent
/// characters are at most about 48 px apart (56 px after CJK punctuation); two spaces are 64 px.
const WORD_GAP_MIN_PX: f32 = 52.0;

/// Greedy CTC decoding of one line: argmax per step, collapse repeats, drop blanks.
/// `probs` is `[t, c]` row-major; `step_px` is the input width covered by one time step
/// (8 for PP-OCRv6). The recognizer often swallows runs of several spaces (table columns,
/// tab-separated text), so a gap between two emitted characters that is much wider than the
/// line's usual character pitch gets a space.
pub(crate) fn ctc_decode(probs: &[f32], t: usize, c: usize, dict: &Dict, step_px: f32) -> Decoded {
    let mut emitted: Vec<(usize, usize)> = Vec::new(); // (time step, class)
    let mut sum = 0.0f32;
    let mut last = 0usize;
    for step in 0..t {
        let row = &probs[step * c..(step + 1) * c];
        let (mut best, mut best_p) = (0usize, f32::MIN);
        for (i, &p) in row.iter().enumerate() {
            if p > best_p {
                best_p = p;
                best = i;
            }
        }
        if best != 0 && best != last {
            emitted.push((step, best));
            sum += best_p;
        }
        last = best;
    }
    let space = dict.chars.len() + 1;
    let mut gaps: Vec<f32> = emitted
        .windows(2)
        .filter(|w| w[0].1 != space && w[1].1 != space)
        .map(|w| (w[1].0 - w[0].0) as f32 * step_px)
        .collect();
    let threshold = if gaps.len() >= 4 {
        gaps.sort_by(|a, b| a.total_cmp(b));
        (WORD_GAP_PITCHES * gaps[(gaps.len() - 1) / 2]).max(WORD_GAP_MIN_PX)
    } else {
        f32::MAX
    };
    let mut text = String::new();
    for (i, &(step, class)) in emitted.iter().enumerate() {
        if i > 0 {
            let (prev_step, prev_class) = emitted[i - 1];
            if prev_class != space
                && class != space
                && (step - prev_step) as f32 * step_px > threshold
            {
                text.push(' ');
            }
        }
        text.push_str(dict.get(class));
    }
    let n = emitted.len();
    Decoded {
        text,
        conf: if n == 0 { 0.0 } else { sum / n as f32 },
    }
}

/// Width (in pixels, at height [`REC_H`]) a crop of `w` x `h` gets, capped at `max_w`.
pub(crate) fn rec_width(w: u32, h: u32, max_w: usize) -> usize {
    let ratio = w as f32 / h.max(1) as f32;
    ((REC_H as f32 * ratio).ceil() as usize).clamp(8, max_w)
}

/// Groups line indices into batches of similar width.
///
/// Lines are sorted by width; a batch is closed when it has `max_batch` lines, when adding the
/// next (wider) line would pad the narrowest member by more than `max_pad_ratio` (padded width
/// is at least `min_width`), or when `lines * padded width` would exceed `max_area` (the
/// recognizer's output is `width / 8 * 18710` floats per line, so this bounds peak memory; a
/// single line is always allowed). Returns each batch's line indices and its padded width.
pub(crate) fn plan_batches(
    widths: &[usize],
    max_batch: usize,
    min_width: usize,
    max_pad_ratio: f32,
    max_area: usize,
) -> Vec<(Vec<usize>, usize)> {
    let mut order: Vec<usize> = (0..widths.len()).collect();
    order.sort_by_key(|&i| widths[i]);
    let max_batch = max_batch.max(1);
    let mut out: Vec<(Vec<usize>, usize)> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for i in order {
        let w = widths[i].max(min_width);
        if let Some(&first) = cur.first() {
            let first_w = widths[first].max(min_width);
            if cur.len() >= max_batch
                || w as f32 > first_w as f32 * max_pad_ratio
                || (cur.len() + 1) * w > max_area
            {
                let width = cur
                    .iter()
                    .map(|&j| widths[j])
                    .max()
                    .unwrap_or(0)
                    .max(min_width);
                out.push((std::mem::take(&mut cur), width));
            }
        }
        cur.push(i);
    }
    if !cur.is_empty() {
        let width = cur
            .iter()
            .map(|&j| widths[j])
            .max()
            .unwrap_or(0)
            .max(min_width);
        out.push((cur, width));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dict {
        Dict::from_metadata("a\nb\nc\n你").unwrap()
    }

    /// One-hot style probability rows for `classes` (value 0.9 at the class).
    fn rows(classes: &[usize], c: usize) -> Vec<f32> {
        let mut v = vec![0.01f32; classes.len() * c];
        for (t, &k) in classes.iter().enumerate() {
            v[t * c + k] = 0.9;
        }
        v
    }

    #[test]
    fn dictionary_layout_blank_entries_space() {
        let d = dict();
        assert_eq!(d.classes(), 6);
        assert_eq!(d.get(0), "");
        assert_eq!(d.get(1), "a");
        assert_eq!(d.get(4), "你");
        assert_eq!(d.get(5), " ");
        assert_eq!(d.get(99), "");
    }

    #[test]
    fn dictionary_file_parsing() {
        let d = Dict::from_file_text("\u{feff}x\r\ny\r\nz\r\n").unwrap();
        assert_eq!(d.classes(), 5);
        assert_eq!(d.get(1), "x");
        assert_eq!(d.get(3), "z");
        assert!(Dict::from_file_text("").is_err());
        assert!(Dict::from_metadata("").is_err());
    }

    #[test]
    fn ctc_collapses_repeats_and_drops_blanks() {
        let d = dict();
        // a a _ a b b _ c  -> "aabc" (the blank separates the two a's)
        let p = rows(&[1, 1, 0, 1, 2, 2, 0, 3], 6);
        let r = ctc_decode(&p, 8, 6, &d, 8.0);
        assert_eq!(r.text, "aabc");
        assert!((r.conf - 0.9).abs() < 1e-6);
    }

    #[test]
    fn ctc_space_cjk_and_empty() {
        let d = dict();
        let r = ctc_decode(&rows(&[4, 0, 5, 0, 1], 6), 5, 6, &d, 8.0);
        assert_eq!(r.text, "你 a");
        let r = ctc_decode(&rows(&[0, 0, 0], 6), 3, 6, &d, 8.0);
        assert_eq!(
            r,
            Decoded {
                text: String::new(),
                conf: 0.0
            }
        );
    }

    #[test]
    fn ctc_confidence_is_mean_of_emitted_steps() {
        let d = dict();
        let mut p = rows(&[1, 2], 6);
        p[6 + 2] = 0.5; // second char only 0.5
        let r = ctc_decode(&p, 2, 6, &d, 8.0);
        assert!((r.conf - 0.7).abs() < 1e-6, "{}", r.conf);
    }

    #[test]
    fn wide_gap_gets_a_space_but_normal_pitch_does_not() {
        let d = dict();
        // Characters every 3 steps (24 px), then a gap of 8 steps (64 px) in the middle.
        let seq = [
            1, 0, 0, 2, 0, 0, 3, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 3, 0, 0, 1,
        ];
        let p = rows(&seq, 6);
        assert_eq!(ctc_decode(&p, seq.len(), 6, &d, 8.0).text, "abca bca");
        // The same gap is not a break when every character is that far apart (wide script).
        let sparse = [
            1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0,
            0, 0, 0, 2,
        ];
        assert_eq!(
            ctc_decode(&rows(&sparse, 6), sparse.len(), 6, &d, 8.0).text,
            "abcab"
        );
        // An emitted space is never doubled, and short lines are left alone.
        let with_space = [1, 0, 0, 2, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 1];
        assert_eq!(
            ctc_decode(&rows(&with_space, 6), with_space.len(), 6, &d, 8.0).text,
            "ab ca"
        );
        let short = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        assert_eq!(
            ctc_decode(&rows(&short, 6), short.len(), 6, &d, 8.0).text,
            "ab"
        );
    }

    #[test]
    fn widths_follow_the_aspect_ratio() {
        assert_eq!(rec_width(480, 24, 4000), 960);
        assert_eq!(rec_width(10, 40, 4000), 12);
        assert_eq!(rec_width(1, 100, 4000), 8);
        assert_eq!(rec_width(100_000, 10, 4000), 4000);
    }

    #[test]
    fn batches_group_similar_widths() {
        let widths = [900, 100, 120, 950, 410, 130];
        let b = plan_batches(&widths, 4, 320, 1.25, usize::MAX);
        // 100, 120, 130 pad to 320 together; 410 alone (410 > 320 * 1.25); 900 and 950 together.
        let sets: Vec<Vec<usize>> = b.iter().map(|(v, _)| v.clone()).collect();
        assert_eq!(sets, vec![vec![1, 2, 5], vec![4], vec![0, 3]]);
        assert_eq!(b[0].1, 320);
        assert_eq!(b[1].1, 410);
        assert_eq!(b[2].1, 950);
    }

    #[test]
    fn batch_size_cap_and_singletons() {
        let widths = [330, 330, 330, 330, 330];
        let b = plan_batches(&widths, 2, 320, 1.25, usize::MAX);
        assert_eq!(
            b.iter().map(|(v, _)| v.len()).collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
        let b = plan_batches(&widths, 1, 320, 1.25, usize::MAX);
        assert_eq!(b.len(), 5);
        assert!(plan_batches(&[], 4, 320, 1.25, usize::MAX).is_empty());
    }

    #[test]
    fn batch_area_budget_bounds_memory() {
        let widths = [1000, 1000, 1000, 1000];
        let b = plan_batches(&widths, 8, 320, 1.25, 2000);
        assert_eq!(
            b.iter().map(|(v, _)| v.len()).collect::<Vec<_>>(),
            vec![2, 2]
        );
        // A single line over budget still runs.
        let b = plan_batches(&[5000], 8, 320, 1.25, 2000);
        assert_eq!(b, vec![(vec![0], 5000)]);
    }
}
