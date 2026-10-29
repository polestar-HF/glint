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

impl Dict {
    /// From the model's `character` metadata (entries separated by `\n`, no trailing newline).
    pub fn from_metadata(s: &str) -> Result<Self> {
        let chars: Vec<Box<str>> = s.split('\n').map(Box::from).collect();
        Self::checked(chars)
    }

    /// From a `ppocr_keys`-style file: one entry per line.
    pub fn from_file_text(s: &str) -> Result<Self> {
        let s = s.strip_prefix('\u{feff}').unwrap_or(s);
        let mut chars: Vec<Box<str>> = s.split('\n').map(|l| Box::from(l.strip_suffix('\r').unwrap_or(l))).collect();
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

/// Greedy CTC decoding of one line: argmax per step, collapse repeats, drop blanks.
/// `probs` is `[t, c]` row-major.
pub(crate) fn ctc_decode(probs: &[f32], t: usize, c: usize, dict: &Dict) -> Decoded {
    let mut text = String::new();
    let mut sum = 0.0f32;
    let mut n = 0u32;
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
            text.push_str(dict.get(best));
            sum += best_p;
            n += 1;
        }
        last = best;
    }
    Decoded { text, conf: if n == 0 { 0.0 } else { sum / n as f32 } }
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
                let width = cur.iter().map(|&j| widths[j]).max().unwrap_or(0).max(min_width);
                out.push((std::mem::take(&mut cur), width));
            }
        }
        cur.push(i);
    }
    if !cur.is_empty() {
        let width = cur.iter().map(|&j| widths[j]).max().unwrap_or(0).max(min_width);
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
        let r = ctc_decode(&p, 8, 6, &d);
        assert_eq!(r.text, "aabc");
        assert!((r.conf - 0.9).abs() < 1e-6);
    }

    #[test]
    fn ctc_space_cjk_and_empty() {
        let d = dict();
        let r = ctc_decode(&rows(&[4, 0, 5, 0, 1], 6), 5, 6, &d);
        assert_eq!(r.text, "你 a");
        let r = ctc_decode(&rows(&[0, 0, 0], 6), 3, 6, &d);
        assert_eq!(r, Decoded { text: String::new(), conf: 0.0 });
    }

    #[test]
    fn ctc_confidence_is_mean_of_emitted_steps() {
        let d = dict();
        let mut p = rows(&[1, 2], 6);
        p[6 + 2] = 0.5; // second char only 0.5
        let r = ctc_decode(&p, 2, 6, &d);
        assert!((r.conf - 0.7).abs() < 1e-6, "{}", r.conf);
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
        let widths = [900, 100, 120, 950, 400, 130];
        let b = plan_batches(&widths, 4, 320, 1.25, usize::MAX);
        // 100, 120, 130 pad to 320 together; 400 alone (400 > 320 * 1.25); 900 and 950 together.
        let sets: Vec<Vec<usize>> = b.iter().map(|(v, _)| v.clone()).collect();
        assert_eq!(sets, vec![vec![1, 2, 5], vec![4], vec![0, 3]]);
        assert_eq!(b[0].1, 320);
        assert_eq!(b[1].1, 400);
        assert_eq!(b[2].1, 950);
    }

    #[test]
    fn batch_size_cap_and_singletons() {
        let widths = [330, 330, 330, 330, 330];
        let b = plan_batches(&widths, 2, 320, 1.25, usize::MAX);
        assert_eq!(b.iter().map(|(v, _)| v.len()).collect::<Vec<_>>(), vec![2, 2, 1]);
        let b = plan_batches(&widths, 1, 320, 1.25, usize::MAX);
        assert_eq!(b.len(), 5);
        assert!(plan_batches(&[], 4, 320, 1.25, usize::MAX).is_empty());
    }

    #[test]
    fn batch_area_budget_bounds_memory() {
        let widths = [1000, 1000, 1000, 1000];
        let b = plan_batches(&widths, 8, 320, 1.25, 2000);
        assert_eq!(b.iter().map(|(v, _)| v.len()).collect::<Vec<_>>(), vec![2, 2]);
        // A single line over budget still runs.
        let b = plan_batches(&[5000], 8, 320, 1.25, 2000);
        assert_eq!(b, vec![(vec![0], 5000)]);
    }
}
