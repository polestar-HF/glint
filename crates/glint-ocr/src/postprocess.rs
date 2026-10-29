//! Pure text post-processing shared by every OCR engine.
//!
//! Nothing in here touches images or ONNX Runtime, so it is cheap to test thoroughly:
//!
//! 1. [`normalize_line`]: control characters, full-width/half-width punctuation that
//!    matches the script around it, stray spaces.
//! 2. [`reading_order`]: recursive XY-cut. Columns are separated first (left to right), then
//!    rows (top to bottom), so a two-column page reads column by column instead of
//!    interleaving the columns line by line.
//! 3. [`paragraphs`]: lines that sit in the same column are clustered into paragraphs by
//!    vertical gap, left alignment, line height, a short last line and terminal
//!    punctuation. CJK lines are joined without a space, Latin lines with one, and
//!    end-of-line hyphenation is undone. Blocks that look like lists or UI labels (many short,
//!    ragged lines) keep their line breaks.
//! 4. [`detect_lang`]: script counting, then `whatlang` restricted to a handful of Latin-script
//!    languages.
//!
//! [`process`] runs all four and is what the engines call.

use crate::OcrLine;

/// Result of [`process`].
#[derive(Debug, Clone)]
pub struct Processed {
    /// Lines in reading order with normalized text. Empty lines are dropped.
    pub lines: Vec<OcrLine>,
    /// Paragraphs (or, with `merge_lines == false`, lines) joined with `\n`.
    pub text: String,
    pub detected_lang: Option<String>,
}

/// Normalizes, orders, merges and classifies raw OCR lines.
pub fn process(lines: Vec<OcrLine>, merge_lines: bool) -> Processed {
    let mut lines: Vec<OcrLine> = lines
        .into_iter()
        .filter_map(|mut l| {
            l.text = normalize_line(&l.text);
            (!l.text.is_empty()).then_some(l)
        })
        .collect();
    let boxes: Vec<[f32; 4]> = lines.iter().map(|l| l.bbox).collect();
    let order = reading_order(&boxes);
    let mut slots: Vec<Option<OcrLine>> = lines.drain(..).map(Some).collect();
    let lines: Vec<OcrLine> = order.into_iter().filter_map(|i| slots[i].take()).collect();

    let text = if merge_lines {
        paragraphs(&lines).join("\n")
    } else {
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    };
    let detected_lang = detect_lang(&text);
    Processed { lines, text, detected_lang }
}

// ---------------------------------------------------------------------------------------
// Character classes
// ---------------------------------------------------------------------------------------

fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FDF | 0x3005 | 0x3007 | 0x3021..=0x3029 | 0x3038..=0x303B
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F)
}

fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF)
}

/// CJK punctuation and full-width punctuation forms (not full-width letters or digits).
fn is_cjk_punct(c: char) -> bool {
    matches!(c as u32,
        0x3001..=0x3004 | 0x3008..=0x3020 | 0x3030 | 0x303D..=0x303F
        | 0xFF01..=0xFF0F | 0xFF1A..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65
        | 0xFFE0..=0xFFE6)
}

/// Characters after which no space is wanted when two lines are joined.
fn is_cjk(c: char) -> bool {
    is_han(c) || is_kana(c) || is_cjk_punct(c)
}

fn is_latin_letter(c: char) -> bool {
    c.is_alphabetic() && matches!(c as u32, 0x41..=0x24F | 0x1E00..=0x1EFF)
}

fn is_cyrillic(c: char) -> bool {
    matches!(c as u32, 0x0400..=0x052F)
}

fn is_arabic(c: char) -> bool {
    matches!(c as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

fn is_thai(c: char) -> bool {
    matches!(c as u32, 0x0E00..=0x0E7F)
}

/// ASCII punctuation that has a full-width counterpart, and the counterpart.
fn to_fullwidth(c: char) -> Option<char> {
    Some(match c {
        ',' => '，',
        '.' => '。',
        '!' => '！',
        '?' => '？',
        ':' => '：',
        ';' => '；',
        '(' => '（',
        ')' => '）',
        _ => return None,
    })
}

fn to_halfwidth(c: char) -> Option<char> {
    Some(match c {
        '，' | '、' => ',',
        '。' => '.',
        '！' => '!',
        '？' => '?',
        '：' => ':',
        '；' => ';',
        '（' => '(',
        '）' => ')',
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------------------

/// Cleans one recognized line.
///
/// * drops control, zero-width and replacement characters; all kinds of spaces become `' '`
/// * full-width letters and digits become ASCII
/// * ASCII punctuation after a CJK character becomes full-width (`你好,世界.` -> `你好，世界。`)
///   and full-width punctuation inside Latin text becomes ASCII (`Hello，world` -> `Hello, world`),
///   decided from the neighbouring characters so mixed text keeps its own style
/// * runs of spaces collapse; spaces between two CJK characters, or next to full-width
///   punctuation, are removed
pub fn normalize_line(s: &str) -> String {
    // Pass 1: character level clean-up.
    let mut chars: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        let c = match c as u32 {
            0xFF10..=0xFF19 | 0xFF21..=0xFF3A | 0xFF41..=0xFF5A => {
                char::from_u32(c as u32 - 0xFF01 + 0x21).unwrap_or(c)
            }
            0x00A0 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000 | 0x0009 => ' ',
            0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x2064 | 0xFEFF | 0xFFFD | 0x00AD => continue,
            _ if c.is_control() => ' ',
            _ => c,
        };
        chars.push(c);
    }

    // Pass 2: punctuation style by context.
    let n = chars.len();
    let prev_sig = |i: usize| chars[..i].iter().rev().find(|c| **c != ' ').copied();
    let next_sig = |i: usize| chars[i + 1..].iter().find(|c| **c != ' ').copied();
    let mut out: Vec<char> = Vec::with_capacity(n + 4);
    for i in 0..n {
        let c = chars[i];
        let prev = prev_sig(i);
        let next = next_sig(i);
        let prev_cjk = prev.is_some_and(|p| is_han(p) || is_kana(p) || is_cjk_punct(p));
        let next_cjk = next.is_some_and(|p| is_han(p) || is_kana(p));
        if let Some(fw) = to_fullwidth(c) {
            let directly_after = i > 0 && chars[i - 1] != ' ';
            let convert = match c {
                // `.` only right after CJK text; "a.com" and "3.14" stay.
                '.' => directly_after && prev.is_some_and(|p| is_han(p) || is_kana(p)),
                '(' => next_cjk && !prev.is_some_and(|p| p.is_ascii_alphanumeric()),
                ')' => directly_after && prev.is_some_and(|p| is_han(p) || is_kana(p)),
                // `:` and `,` between digits are times / numbers.
                ':' | ',' => {
                    prev.is_some_and(|p| is_han(p) || is_kana(p) || is_cjk_punct(p))
                        && !(prev.is_some_and(|p| p.is_ascii_digit())
                            && next.is_some_and(|p| p.is_ascii_digit()))
                }
                _ => prev_cjk && directly_after,
            };
            out.push(if convert { fw } else { c });
            continue;
        }
        if let Some(hw) = to_halfwidth(c) {
            let latin_before = prev.is_some_and(|p| p.is_ascii_alphanumeric() || p == '.' || p == ',');
            let to_ascii = match c {
                '（' => !prev_cjk && next.is_some_and(|p| p.is_ascii_alphanumeric()),
                '）' => latin_before && !next_cjk,
                _ => latin_before && !next_cjk,
            };
            if to_ascii {
                out.push(hw);
                // A sentence mark needs a space before the next word, but not inside "3。14".
                let between_digits = prev.is_some_and(|p| p.is_ascii_digit())
                    && next.is_some_and(|p| p.is_ascii_digit());
                if matches!(hw, ',' | '.' | '!' | '?' | ':' | ';')
                    && !between_digits
                    && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric())
                {
                    out.push(' ');
                }
            } else {
                out.push(c);
            }
            continue;
        }
        out.push(c);
    }

    // Pass 3: spaces.
    let mut res = String::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        let c = out[i];
        if c != ' ' {
            res.push(c);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < out.len() && out[j] == ' ' {
            j += 1;
        }
        let before = res.chars().next_back();
        let after = out.get(j).copied();
        let drop = match (before, after) {
            (None, _) | (_, None) => true,
            (Some(b), Some(a)) => {
                let both_cjk = (is_han(b) || is_kana(b)) && (is_han(a) || is_kana(a));
                let near_fw_punct = (is_cjk_punct(b) && !matches!(b, '（' | '「' | '『' | '【' | '《' | '〈'))
                    || (is_cjk_punct(a) && !matches!(a, '（' | '「' | '『' | '【' | '《' | '〈'));
                both_cjk || near_fw_punct
            }
        };
        if !drop {
            res.push(' ');
        }
        i = j;
    }
    res
}

// ---------------------------------------------------------------------------------------
// Reading order
// ---------------------------------------------------------------------------------------

/// A vertical gap (column gutter) must be at least this many median line heights.
const COLUMN_GAP_FACTOR: f32 = 1.0;
/// Trim applied to both ends of a line's vertical extent before looking for row gaps; DB
/// boxes are inflated, so neighbouring lines usually touch or overlap.
const ROW_TRIM: f32 = 0.2;

#[derive(Clone, Copy, Debug)]
struct Rc {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Rc {
    fn from_box(b: &[f32; 4]) -> Self {
        Rc { x0: b[0], y0: b[1], x1: b[0] + b[2].max(0.0), y1: b[1] + b[3].max(0.0) }
    }
    fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    fn cy(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[(v.len() - 1) / 2]
}

/// Returns the indices of `boxes` (`[x, y, w, h]`) in reading order.
///
/// Recursive XY-cut: if the group has a clear empty vertical corridor (a column gutter of at
/// least one line height, with both sides overlapping vertically) it is split there and the left
/// part is read before the right one. Otherwise it is split at the widest empty horizontal gap
/// and the upper part is read first. What cannot be split any further is a single row and is
/// read left to right.
pub fn reading_order(boxes: &[[f32; 4]]) -> Vec<usize> {
    let n = boxes.len();
    if n <= 1 {
        return (0..n).collect();
    }
    let rects: Vec<Rc> = boxes.iter().map(Rc::from_box).collect();
    let mut hs: Vec<f32> = rects.iter().map(Rc::h).filter(|h| *h > 0.0).collect();
    let med_h = median(&mut hs).max(1.0);
    let mut out = Vec::with_capacity(n);
    xy_cut(&rects, (0..n).collect(), med_h, &mut out);
    out
}

fn xy_cut(rects: &[Rc], idx: Vec<usize>, med_h: f32, out: &mut Vec<usize>) {
    if idx.len() <= 1 {
        out.extend(idx);
        return;
    }
    if let Some(cut) = widest_gap(rects, &idx, true, COLUMN_GAP_FACTOR * med_h) {
        let (left, right): (Vec<usize>, Vec<usize>) = idx.iter().partition(|&&i| rects[i].x0 < cut);
        if !left.is_empty() && !right.is_empty() && overlap_vertically(rects, &left, &right) {
            xy_cut(rects, left, med_h, out);
            xy_cut(rects, right, med_h, out);
            return;
        }
    }
    if let Some(cut) = widest_gap(rects, &idx, false, 0.0) {
        let (top, bottom): (Vec<usize>, Vec<usize>) =
            idx.iter().partition(|&&i| trimmed_y(&rects[i]).0 < cut);
        if !top.is_empty() && !bottom.is_empty() {
            xy_cut(rects, top, med_h, out);
            xy_cut(rects, bottom, med_h, out);
            return;
        }
    }
    // One row (or an arrangement that cannot be cut): group by vertical centre, then left to right.
    let mut idx = idx;
    idx.sort_by(|&a, &b| rects[a].cy().total_cmp(&rects[b].cy()));
    let mut k = 0;
    while k < idx.len() {
        let anchor = rects[idx[k]].cy();
        let mut e = k + 1;
        while e < idx.len() && rects[idx[e]].cy() - anchor < 0.5 * med_h {
            e += 1;
        }
        idx[k..e].sort_by(|&a, &b| rects[a].x0.total_cmp(&rects[b].x0));
        out.extend_from_slice(&idx[k..e]);
        k = e;
    }
}

fn trimmed_y(r: &Rc) -> (f32, f32) {
    let t = ROW_TRIM * r.h().max(0.0);
    (r.y0 + t, r.y1 - t)
}

/// Midpoint of the widest empty corridor along one axis that is wider than `min_gap`.
fn widest_gap(rects: &[Rc], idx: &[usize], along_x: bool, min_gap: f32) -> Option<f32> {
    let mut iv: Vec<(f32, f32)> = idx
        .iter()
        .map(|&i| if along_x { (rects[i].x0, rects[i].x1) } else { trimmed_y(&rects[i]) })
        .collect();
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = iv[0].1;
    let mut best = min_gap;
    let mut cut = None;
    for &(s, e) in &iv[1..] {
        if s - reach > best {
            best = s - reach;
            cut = Some((reach + s) * 0.5);
        }
        reach = reach.max(e);
    }
    cut
}

/// True when the vertical extents of the two groups overlap by at least half of the shorter one,
/// i.e. they really are side by side and not two blocks stacked at different indents.
fn overlap_vertically(rects: &[Rc], a: &[usize], b: &[usize]) -> bool {
    let ext = |g: &[usize]| {
        g.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &i| (lo.min(rects[i].y0), hi.max(rects[i].y1)))
    };
    let (a0, a1) = ext(a);
    let (b0, b1) = ext(b);
    let overlap = a1.min(b1) - a0.max(b0);
    overlap >= 0.5 * (a1 - a0).min(b1 - b0)
}

// ---------------------------------------------------------------------------------------
// Paragraph merging
// ---------------------------------------------------------------------------------------

/// Vertical gap above which two lines start different paragraphs, in line heights, when there
/// are too few gaps in the block to learn the normal line spacing from.
const PARA_GAP_FALLBACK: f32 = 0.8;
/// Lines whose height differs by more than this ratio are different kinds of text.
const MIN_HEIGHT_RATIO: f32 = 0.65;
/// A line that stops this far (fraction of block width) before the block's right edge ends a paragraph.
const SHORT_LINE_SLACK: f32 = 0.30;

struct Ln<'a> {
    text: &'a str,
    l: f32,
    r: f32,
    t: f32,
    b: f32,
}

impl Ln<'_> {
    fn h(&self) -> f32 {
        (self.b - self.t).max(1.0)
    }
    fn cx(&self) -> f32 {
        (self.l + self.r) * 0.5
    }
    fn cy(&self) -> f32 {
        (self.t + self.b) * 0.5
    }
}

/// Merges lines (already in reading order and normalized) into paragraphs.
pub fn paragraphs(lines: &[OcrLine]) -> Vec<String> {
    let lns: Vec<Ln> = lines
        .iter()
        .map(|l| Ln { text: &l.text, l: l.bbox[0], r: l.bbox[0] + l.bbox[2], t: l.bbox[1], b: l.bbox[1] + l.bbox[3] })
        .collect();
    let mut out = Vec::new();
    for block in split_blocks(&lns) {
        merge_block(&lns[block.0..block.1], &mut out);
    }
    out
}

/// Splits the ordered lines into blocks, i.e. runs that continue downwards in the same column.
/// Returns half-open index ranges.
fn split_blocks(l: &[Ln]) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut start = 0;
    for i in 1..l.len() {
        let (a, b) = (&l[i - 1], &l[i]);
        let h = a.h().min(b.h());
        let jumps_up = b.cy() < a.cy() - 0.5 * h;
        let side_by_side = (b.cy() - a.cy()).abs() < 0.5 * h;
        let x_overlap = a.r.min(b.r) - a.l.max(b.l);
        let far = b.t - a.b > 2.5 * a.h().max(b.h());
        if jumps_up || side_by_side || x_overlap <= 0.0 || far {
            blocks.push((start, i));
            start = i;
        }
    }
    if start < l.len() {
        blocks.push((start, l.len()));
    }
    blocks
}

fn merge_block(b: &[Ln], out: &mut Vec<String>) {
    if b.is_empty() {
        return;
    }
    if b.len() == 1 || is_list_like(b) {
        out.extend(b.iter().map(|l| l.text.to_string()));
        return;
    }
    let mut hs: Vec<f32> = b.iter().map(Ln::h).collect();
    let bh = median(&mut hs);
    let left = b.iter().map(|l| l.l).fold(f32::MAX, f32::min);
    let right = b.iter().map(|l| l.r).fold(f32::MIN, f32::max);
    let width = (right - left).max(1.0);
    let tol = 0.9 * bh;

    // Normal line spacing, learned from the block itself.
    let mut gaps: Vec<f32> = b.windows(2).map(|w| (w[1].t - w[0].b).max(0.0)).collect();
    let para_gap = if gaps.len() >= 2 {
        (0.55 * bh).max(1.8 * median(&mut gaps))
    } else {
        PARA_GAP_FALLBACK * bh
    };

    #[derive(PartialEq, Clone, Copy)]
    enum Align {
        Left,
        Center,
    }
    let mut cur = b[0].text.to_string();
    let mut n_in_para = 1;
    let mut body_left = b[0].l;
    let mut align = Align::Left;
    let mut para_marker = starts_with_marker(b[0].text);

    for w in b.windows(2) {
        let (a, n) = (&w[0], &w[1]);
        let ratio = a.h().min(n.h()) / a.h().max(n.h());
        let gap = n.t - a.b;
        let slack = right - a.r;

        // Decide how `n` is aligned relative to the paragraph so far.
        let mut new_align = align;
        let mut new_body_left = body_left;
        let aligned = if n_in_para == 1 {
            let same_left = (n.l - a.l).abs() <= tol;
            let indented_first = a.l - n.l >= tol && a.l - n.l <= 4.0 * bh;
            let hanging = para_marker && n.l - a.l >= tol && n.l - a.l <= 3.0 * bh;
            if same_left || indented_first || hanging {
                new_align = Align::Left;
                new_body_left = n.l;
                true
            } else if (n.cx() - a.cx()).abs() <= tol {
                new_align = Align::Center;
                true
            } else {
                false
            }
        } else if align == Align::Left {
            (n.l - body_left).abs() <= tol
        } else {
            (n.cx() - a.cx()).abs() <= tol
        };

        let ends = if !aligned {
            true
        } else if starts_with_marker(n.text) || ratio < MIN_HEIGHT_RATIO || gap > para_gap {
            true
        } else if new_align == Align::Center || align == Align::Center {
            ends_with_terminal(a.text)
        } else {
            slack > SHORT_LINE_SLACK * width
                || (ends_with_terminal(a.text) && slack > (1.5 * bh).max(0.08 * width))
        };

        if ends {
            out.push(std::mem::take(&mut cur));
            cur.push_str(n.text);
            n_in_para = 1;
            body_left = n.l;
            align = Align::Left;
            para_marker = starts_with_marker(n.text);
        } else {
            join_lines(&mut cur, n.text);
            n_in_para += 1;
            align = new_align;
            body_left = new_body_left;
        }
    }
    out.push(cur);
}

/// "Many short, uniform lines": a menu, a settings list, song lyrics, a chat log. Real paragraphs
/// have lines that (all but the last) reach close to the block's right edge.
fn is_list_like(b: &[Ln]) -> bool {
    if b.len() < 3 {
        return false;
    }
    let left = b.iter().map(|l| l.l).fold(f32::MAX, f32::min);
    let right = b.iter().map(|l| l.r).fold(f32::MIN, f32::max);
    let width = (right - left).max(1.0);
    let body = &b[..b.len() - 1];
    let short = body.iter().filter(|l| right - l.r > 0.25 * width).count();
    // One short line is just the end of a paragraph; several are a list.
    if short >= 2 && short * 5 >= body.len() * 2 {
        return true;
    }
    // A column of items that each end like a sentence is a list of sentences, not one paragraph.
    let marked = b.iter().filter(|l| starts_with_marker(l.text)).count();
    marked * 2 >= b.len()
}

/// Terminal punctuation: `。！？.!?:：` (and `…`, `;`-like marks), ignoring closing quotes/brackets.
fn ends_with_terminal(s: &str) -> bool {
    let t = s.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '”' | '’' | '」' | '』' | '）' | ')' | ']' | '】' | '》' | '*')
    });
    t.chars().next_back().is_some_and(|c| matches!(c, '。' | '！' | '？' | '.' | '!' | '?' | ':' | '：' | '…' | '；' | ';'))
}

/// Bullets and numbering that start a new item.
fn starts_with_marker(s: &str) -> bool {
    let mut it = s.chars();
    let Some(c0) = it.next() else { return false };
    let rest: &str = it.as_str();
    if matches!(c0, '•' | '·' | '●' | '○' | '◦' | '▪' | '■' | '□' | '◆' | '◇' | '▶' | '▸' | '►' | '‣' | '⁃') {
        return true;
    }
    if matches!(c0, '-' | '–' | '—' | '*' | '+' | '>') {
        return rest.starts_with(' ');
    }
    if matches!(c0 as u32, 0x2460..=0x2473 | 0x2776..=0x277F) {
        return true;
    }
    if c0 == '（' || c0 == '(' {
        let inner: String = rest.chars().take_while(|c| c.is_ascii_digit() || is_han(*c)).collect();
        return !inner.is_empty()
            && inner.chars().count() <= 3
            && matches!(rest[inner.len()..].chars().next(), Some(')') | Some('）'));
    }
    if c0.is_ascii_digit() {
        let digits = 1 + rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits <= 3 {
            let after = &s[digits..];
            let mut a = after.chars();
            return match a.next() {
                Some('.') | Some(')') => a.next() == Some(' '),
                Some('、') => true,
                _ => false,
            };
        }
    }
    false
}

/// Appends `next` to the paragraph text `cur` as a continuation line.
///
/// * `transla-` + `tion` becomes `translation` (hyphen dropped when the next word is lower case)
/// * either side CJK: no space
/// * everything else: one space
fn join_lines(cur: &mut String, next: &str) {
    let next = next.trim_start();
    let Some(last) = cur.chars().next_back() else {
        cur.push_str(next);
        return;
    };
    let Some(first) = next.chars().next() else { return };
    if matches!(last, '-' | '\u{2010}' | '\u{2011}' | '\u{00AD}') {
        let before = cur.chars().rev().nth(1);
        if before.is_some_and(|c| c.is_alphabetic() && !is_han(c) && !is_kana(c)) && first.is_alphanumeric() {
            if first.is_lowercase() {
                cur.pop();
            }
            cur.push_str(next);
            return;
        }
    }
    let no_space = is_cjk(last) || (is_han(first) || is_kana(first) || is_cjk_punct(first)) || last == ' ';
    if !no_space {
        cur.push(' ');
    }
    cur.push_str(next);
}

// ---------------------------------------------------------------------------------------
// Language guess
// ---------------------------------------------------------------------------------------

/// Cheap language guess: `ja` / `ko` / `zh` from the scripts present, `ru` / `th` / `ar` from
/// Cyrillic / Thai / Arabic, and `whatlang` restricted to `en fr de es pt it vi` for Latin text.
/// Returns `None` when there is no letter at all, and `"en"` when the Latin guess is unsure.
pub fn detect_lang(text: &str) -> Option<String> {
    let (mut han, mut kana, mut hangul, mut latin, mut cyr, mut ara, mut thai) = (0usize, 0, 0, 0, 0, 0, 0);
    for c in text.chars() {
        if is_han(c) {
            han += 1;
        } else if is_kana(c) {
            kana += 1;
        } else if is_hangul(c) {
            hangul += 1;
        } else if is_latin_letter(c) {
            latin += 1;
        } else if is_cyrillic(c) {
            cyr += 1;
        } else if is_arabic(c) {
            ara += 1;
        } else if is_thai(c) {
            thai += 1;
        }
    }
    let cjk = han + kana + hangul;
    let letters = cjk + latin + cyr + ara + thai;
    if letters == 0 {
        return None;
    }
    // A CJK character carries far more than a Latin letter, so a few of them are enough.
    if cjk * 4 >= latin && cjk * 2 >= cyr + ara + thai {
        let lang = if kana > 0 && kana * 20 >= han + kana {
            "ja"
        } else if hangul > 0 && hangul * 2 >= han + kana {
            "ko"
        } else {
            "zh"
        };
        return Some(lang.into());
    }
    for (count, code) in [(thai, "th"), (ara, "ar"), (cyr, "ru")] {
        if count * 2 >= letters {
            return Some(code.into());
        }
    }
    if latin == 0 {
        return None;
    }
    Some(detect_latin(text).into())
}

fn detect_latin(text: &str) -> &'static str {
    use whatlang::{Detector, Lang};
    if text.chars().filter(|c| c.is_alphabetic()).count() < 12 {
        return "en";
    }
    let det = Detector::with_allowlist(vec![
        Lang::Eng,
        Lang::Fra,
        Lang::Deu,
        Lang::Spa,
        Lang::Por,
        Lang::Ita,
        Lang::Vie,
    ]);
    match det.detect(text) {
        Some(info) if info.is_reliable() || info.confidence() >= 0.5 => match info.lang() {
            Lang::Fra => "fr",
            Lang::Deu => "de",
            Lang::Spa => "es",
            Lang::Por => "pt",
            Lang::Ita => "it",
            Lang::Vie => "vi",
            _ => "en",
        },
        _ => "en",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ln(text: &str, x: f32, y: f32, w: f32, h: f32) -> OcrLine {
        OcrLine { text: text.into(), bbox: [x, y, w, h], confidence: 0.99 }
    }

    /// Lines of equal height 20 stacked with `pitch`, each as wide as its text (8 px per char).
    fn stack(texts: &[&str], x: f32, y: f32, pitch: f32) -> Vec<OcrLine> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| ln(t, x, y + pitch * i as f32, 8.0 * t.chars().count() as f32, 20.0))
            .collect()
    }

    fn texts(lines: &[OcrLine]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    // ----- normalization -----

    #[test]
    fn normalize_cjk_punctuation_becomes_fullwidth() {
        assert_eq!(normalize_line("你好,世界."), "你好，世界。");
        assert_eq!(normalize_line("真的吗?是的!"), "真的吗？是的！");
        assert_eq!(normalize_line("注意:请看(图一)"), "注意：请看（图一）");
    }

    #[test]
    fn normalize_keeps_numbers_urls_and_versions() {
        assert_eq!(normalize_line("版本 0.1.0 发布"), "版本 0.1.0 发布");
        assert_eq!(normalize_line("时间 12:30 开始"), "时间 12:30 开始");
        assert_eq!(normalize_line("访问 github.com 获取"), "访问 github.com 获取");
        assert_eq!(normalize_line("共3.14万"), "共3.14万");
        assert_eq!(normalize_line("共 1,000 人"), "共 1,000 人");
    }

    #[test]
    fn normalize_latin_text_gets_ascii_punctuation() {
        assert_eq!(normalize_line("Hello，world。"), "Hello, world.");
        assert_eq!(normalize_line("Really？Yes！"), "Really? Yes!");
        assert_eq!(normalize_line("Total：5"), "Total: 5");
        assert_eq!(normalize_line("pi is 3。14"), "pi is 3.14");
    }

    #[test]
    fn normalize_mixed_text_keeps_chinese_style() {
        // Full-width comma between a Latin word and Chinese text stays full-width.
        assert_eq!(normalize_line("支持 Windows，默认"), "支持 Windows，默认");
        assert_eq!(normalize_line("Alt+Q，可以修改。"), "Alt+Q，可以修改。");
    }

    #[test]
    fn normalize_fullwidth_alnum_and_spaces() {
        assert_eq!(normalize_line("ＡＢＣ１２３"), "ABC123");
        assert_eq!(normalize_line("a\u{3000}b\u{00A0} c  d"), "a b c d");
        assert_eq!(normalize_line("  trim me \t"), "trim me");
        assert_eq!(normalize_line("zero\u{200B}width\u{FFFD}"), "zerowidth");
    }

    #[test]
    fn normalize_removes_spaces_inside_cjk_only() {
        assert_eq!(normalize_line("识 别 结 果"), "识别结果");
        assert_eq!(normalize_line("你好 ， 世界 。"), "你好，世界。");
        assert_eq!(normalize_line("日本語 English 混在"), "日本語 English 混在");
        assert_eq!(normalize_line("한국어 문장"), "한국어 문장");
    }

    // ----- reading order -----

    fn order_of(lines: &[OcrLine]) -> Vec<usize> {
        reading_order(&lines.iter().map(|l| l.bbox).collect::<Vec<_>>())
    }

    #[test]
    fn order_single_column_top_to_bottom_even_if_shuffled() {
        let mut v = stack(&["a", "b", "c", "d"], 10.0, 10.0, 24.0);
        v.swap(0, 3);
        v.swap(1, 2);
        let ord = order_of(&v);
        let got: Vec<&str> = ord.iter().map(|&i| v[i].text.as_str()).collect();
        assert_eq!(got, ["a", "b", "c", "d"]);
    }

    #[test]
    fn order_two_columns_reads_left_column_first() {
        // Rows are interleaved the way a detector sorted by y would return them.
        let left = ["L1 text text", "L2 text text", "L3 text text"];
        let right = ["R1 text text", "R2 text text", "R3 text text"];
        let l = stack(&left, 20.0, 10.0, 22.0);
        let r = stack(&right, 320.0, 10.0, 22.0);
        let mut all = Vec::new();
        for i in 0..3 {
            all.push(l[i].clone());
            all.push(r[i].clone());
        }
        let ord = order_of(&all);
        let got: Vec<&str> = ord.iter().map(|&i| all[i].text.as_str()).collect();
        assert_eq!(got, ["L1 text text", "L2 text text", "L3 text text", "R1 text text", "R2 text text", "R3 text text"]);
    }

    #[test]
    fn order_title_above_two_columns() {
        let mut all = vec![ln("A wide title across the page", 20.0, 5.0, 560.0, 22.0)];
        all.extend(stack(&["left one", "left two"], 20.0, 50.0, 22.0));
        all.extend(stack(&["right one", "right two"], 320.0, 50.0, 22.0));
        // Shuffle: right column first.
        all.rotate_left(3);
        let ord = order_of(&all);
        let got: Vec<&str> = ord.iter().map(|&i| all[i].text.as_str()).collect();
        assert_eq!(got, ["A wide title across the page", "left one", "left two", "right one", "right two"]);
    }

    #[test]
    fn order_three_columns() {
        let mut all = Vec::new();
        for (c, x) in [("a", 10.0), ("b", 210.0), ("c", 410.0)] {
            all.extend(stack(&[&format!("{c}1 some words"), &format!("{c}2 some words")], x, 10.0, 22.0));
        }
        all.reverse();
        let ord = order_of(&all);
        let got: Vec<&str> = ord.iter().map(|&i| all[i].text.as_str()).collect();
        assert_eq!(got, ["a1 some words", "a2 some words", "b1 some words", "b2 some words", "c1 some words", "c2 some words"]);
    }

    #[test]
    fn order_same_row_boxes_go_left_to_right() {
        let v = vec![ln("Cancel", 120.0, 10.0, 50.0, 20.0), ln("OK", 20.0, 11.0, 30.0, 20.0)];
        assert_eq!(order_of(&v), vec![1, 0]);
    }

    #[test]
    fn order_stacked_blocks_at_different_indents_stay_top_down() {
        // A narrow note top-right and a block bottom-left must not be read bottom-left first.
        let v = vec![ln("bottom left block", 10.0, 100.0, 150.0, 20.0), ln("top right note", 300.0, 10.0, 120.0, 20.0)];
        assert_eq!(order_of(&v), vec![1, 0]);
    }

    #[test]
    fn order_handles_tiny_inputs_and_overlapping_boxes() {
        assert!(order_of(&[]).is_empty());
        assert_eq!(order_of(&[ln("x", 0.0, 0.0, 10.0, 10.0)]), vec![0]);
        // Inflated boxes that overlap vertically still read top-down.
        let v = vec![ln("two", 10.0, 22.0, 100.0, 24.0), ln("one", 10.0, 0.0, 100.0, 24.0), ln("three", 10.0, 44.0, 100.0, 24.0)];
        assert_eq!(order_of(&v), vec![1, 0, 2]);
    }

    // ----- paragraphs -----

    fn merged(lines: Vec<OcrLine>) -> Vec<String> {
        let p = process(lines, true);
        p.text.split('\n').map(str::to_string).collect()
    }

    #[test]
    fn paragraph_latin_lines_join_with_space() {
        let v = stack(
            &["The quick brown fox jumps over", "the lazy dog and keeps running", "until it reaches the river bank", "and stops."],
            10.0,
            10.0,
            24.0,
        );
        assert_eq!(
            merged(v),
            ["The quick brown fox jumps over the lazy dog and keeps running until it reaches the river bank and stops."]
        );
    }

    #[test]
    fn paragraph_undoes_hyphenation() {
        let v = stack(
            &["Select any region and the transla-", "tion appears right next to it, so you", "never have to switch windows or retype."],
            10.0,
            10.0,
            24.0,
        );
        let got = merged(v);
        assert_eq!(got, ["Select any region and the translation appears right next to it, so you never have to switch windows or retype."]);
    }

    #[test]
    fn hyphen_before_capital_or_digit_is_kept() {
        let mut s = String::from("an Anglo-");
        join_lines(&mut s, "Saxon king");
        assert_eq!(s, "an Anglo-Saxon king");
        let mut s = String::from("COVID-");
        join_lines(&mut s, "19 cases");
        assert_eq!(s, "COVID-19 cases");
        // A dash that is not part of a word is not touched.
        let mut s = String::from("wait -");
        join_lines(&mut s, "then go");
        assert_eq!(s, "wait - then go");
    }

    #[test]
    fn paragraph_cjk_lines_join_without_space() {
        let v = vec![
            ln("截图翻译工具可以在几毫秒内识别屏幕上的文字，", 10.0, 10.0, 480.0, 22.0),
            ln("译文便会显示在旁边，无需切换窗口。", 10.0, 34.0, 400.0, 22.0),
        ];
        assert_eq!(merged(v), ["截图翻译工具可以在几毫秒内识别屏幕上的文字，译文便会显示在旁边，无需切换窗口。"]);
    }

    #[test]
    fn paragraph_cjk_then_latin_no_space_latin_then_cjk_no_space() {
        let v = vec![
            ln("默认快捷键为", 10.0, 10.0, 400.0, 22.0),
            ln("Alt+Q，可以在设置中修改", 10.0, 34.0, 400.0, 22.0),
        ];
        assert_eq!(merged(v), ["默认快捷键为Alt+Q，可以在设置中修改"]);
    }

    #[test]
    fn paragraph_gap_starts_new_paragraph() {
        // Same width lines; third line is separated by a blank line's worth of space.
        let mut v = stack(&["First paragraph line one is", "long enough to wrap around."], 10.0, 10.0, 24.0);
        v.extend(stack(&["Second paragraph starts here and", "wraps as well to the next line."], 10.0, 80.0, 24.0));
        let got = merged(v);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].starts_with("First") && got[0].ends_with("around."));
        assert!(got[1].starts_with("Second") && got[1].ends_with("line."));
    }

    #[test]
    fn paragraph_ends_at_short_line_with_period() {
        // Both lines are on the same grid; the first is short and ends with a period.
        let v = vec![
            ln("A full line of text in the paragraph that runs wide", 10.0, 10.0, 420.0, 20.0),
            ln("Done.", 10.0, 32.0, 40.0, 20.0),
            ln("Next paragraph starts without extra gap", 10.0, 54.0, 320.0, 20.0),
        ];
        let got = merged(v);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], "A full line of text in the paragraph that runs wide Done.");
        assert_eq!(got[1], "Next paragraph starts without extra gap");
    }

    #[test]
    fn paragraph_full_line_ending_with_period_does_not_end() {
        let v = vec![
            ln("This line ends a sentence right at the margin.", 10.0, 10.0, 400.0, 20.0),
            ln("The next sentence continues in the same block", 10.0, 32.0, 396.0, 20.0),
            ln("of text and ends here.", 10.0, 54.0, 180.0, 20.0),
        ];
        assert_eq!(
            merged(v),
            ["This line ends a sentence right at the margin. The next sentence continues in the same block of text and ends here."]
        );
    }

    #[test]
    fn paragraph_first_line_indent_is_accepted() {
        let v = vec![
            ln("Indented first line of a paragraph that is long", 40.0, 10.0, 380.0, 20.0),
            ln("and the second line starts at the left margin and", 10.0, 32.0, 410.0, 20.0),
            ln("the third line too.", 10.0, 54.0, 150.0, 20.0),
        ];
        assert_eq!(
            merged(v),
            ["Indented first line of a paragraph that is long and the second line starts at the left margin and the third line too."]
        );
    }

    #[test]
    fn list_of_short_lines_keeps_line_breaks() {
        let v = stack(&["General", "Appearance", "Keyboard Shortcuts", "Translation Engines", "About Glint"], 16.0, 14.0, 27.0);
        assert_eq!(merged(v), ["General", "Appearance", "Keyboard Shortcuts", "Translation Engines", "About Glint"]);
    }

    #[test]
    fn tight_uniform_ui_list_stays_line_mode() {
        // Tightly spaced menu entries (no visible gaps) must still not be glued into a sentence.
        let v = stack(&["File", "Edit", "Selection", "View", "Go", "Run"], 10.0, 10.0, 20.0);
        assert_eq!(merged(v).len(), 6);
    }

    #[test]
    fn bullets_start_new_paragraphs() {
        let v = vec![
            ln("Features of this release include", 10.0, 10.0, 300.0, 20.0),
            ln("• Faster offline recognition on all", 10.0, 32.0, 300.0, 20.0),
            ln("• Better paragraph merging for text", 10.0, 54.0, 300.0, 20.0),
        ];
        let got = merged(v);
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[test]
    fn numbered_items_are_markers() {
        assert!(starts_with_marker("1. First"));
        assert!(starts_with_marker("12) Twelfth"));
        assert!(starts_with_marker("3、第三"));
        assert!(starts_with_marker("• item"));
        assert!(starts_with_marker("- item"));
        assert!(starts_with_marker("（1）内容"));
        assert!(!starts_with_marker("2026 was a year"));
        assert!(!starts_with_marker("-5 degrees"));
        assert!(!starts_with_marker("1.5 liters"));
    }

    #[test]
    fn different_font_sizes_do_not_merge() {
        let v = vec![
            ln("A big heading", 10.0, 10.0, 200.0, 40.0),
            ln("small body text under the heading that wraps", 10.0, 54.0, 400.0, 20.0),
            ln("onto a second line of body text here today", 10.0, 76.0, 395.0, 20.0),
        ];
        let got = merged(v);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], "A big heading");
    }

    #[test]
    fn two_column_text_merges_per_column_not_across() {
        let l = vec![
            ln("Fast offline recognition", 15.0, 10.0, 168.0, 22.0),
            ln("keeps every screenshot on", 15.0, 32.0, 198.0, 22.0),
            ln("your own computer, with no", 15.0, 54.0, 202.0, 22.0),
            ln("upload and no account.", 15.0, 76.0, 172.0, 22.0),
        ];
        let r = vec![
            ln("Translate with the engine", 324.0, 10.0, 182.0, 22.0),
            ln("you trust: a free web service,", 323.0, 32.0, 210.0, 22.0),
            ln("your own API key, or a local", 323.0, 54.0, 202.0, 22.0),
            ln("model running on this PC.", 323.0, 76.0, 188.0, 22.0),
        ];
        let mut all = Vec::new();
        for i in 0..4 {
            all.push(r[i].clone());
            all.push(l[i].clone());
        }
        let got = merged(all);
        assert_eq!(
            got,
            [
                "Fast offline recognition keeps every screenshot on your own computer, with no upload and no account.",
                "Translate with the engine you trust: a free web service, your own API key, or a local model running on this PC."
            ]
        );
    }

    #[test]
    fn centered_subtitle_lines_merge() {
        let v = vec![
            ln("I think we should leave", 200.0, 10.0, 200.0, 22.0),
            ln("before it gets dark outside", 187.0, 34.0, 226.0, 22.0),
        ];
        assert_eq!(merged(v), ["I think we should leave before it gets dark outside"]);
    }

    #[test]
    fn centered_subtitle_with_period_keeps_both_sentences_apart() {
        let v = vec![
            ln("It is getting late.", 220.0, 10.0, 160.0, 22.0),
            ln("Let us go home now, please.", 185.0, 34.0, 230.0, 22.0),
        ];
        assert_eq!(merged(v).len(), 2);
    }

    #[test]
    fn merge_false_keeps_every_line() {
        let v = stack(&["one two three four five", "six seven eight nine ten", "eleven twelve."], 10.0, 10.0, 24.0);
        let p = process(v, false);
        assert_eq!(p.text, "one two three four five\nsix seven eight nine ten\neleven twelve.");
        assert_eq!(p.lines.len(), 3);
    }

    #[test]
    fn process_orders_normalizes_and_drops_empty() {
        let v = vec![
            ln("世界,", 10.0, 40.0, 100.0, 20.0),
            ln("   ", 10.0, 90.0, 10.0, 20.0),
            ln("你好", 10.0, 10.0, 100.0, 20.0),
        ];
        let p = process(v, false);
        assert_eq!(texts(&p.lines), ["你好", "世界，"]);
        assert_eq!(p.detected_lang.as_deref(), Some("zh"));
    }

    // ----- language -----

    #[test]
    fn lang_scripts() {
        assert_eq!(detect_lang("截图翻译工具").as_deref(), Some("zh"));
        assert_eq!(detect_lang("スクリーンショットから文字を読み取る").as_deref(), Some("ja"));
        assert_eq!(detect_lang("ひらがなとカタカナ、そして漢字").as_deref(), Some("ja"));
        assert_eq!(detect_lang("한국어 문장입니다").as_deref(), Some("ko"));
        assert_eq!(detect_lang("Привет, как дела у тебя сегодня").as_deref(), Some("ru"));
        assert_eq!(detect_lang("สวัสดีครับ ยินดีที่ได้รู้จัก").as_deref(), Some("th"));
        assert_eq!(detect_lang("مرحبا بالعالم كيف حالك").as_deref(), Some("ar"));
    }

    #[test]
    fn lang_mixed_chinese_english_is_chinese() {
        assert_eq!(detect_lang("Glint 支持 Windows 10 和 Windows 11 系统，默认快捷键为 Alt+Q").as_deref(), Some("zh"));
    }

    #[test]
    fn lang_english_with_a_few_cjk_characters_is_english() {
        assert_eq!(
            detect_lang("Press the button labelled 确定 to continue with the installation of the program").as_deref(),
            Some("en")
        );
    }

    #[test]
    fn lang_single_stray_kana_in_chinese_stays_chinese() {
        let zh = "这是一个很长的中文句子，里面有一个假名の字符，但整体仍然是中文文本，用来测试语言检测的稳定性。";
        assert_eq!(detect_lang(zh).as_deref(), Some("zh"));
    }

    #[test]
    fn lang_latin_languages() {
        assert_eq!(detect_lang("The quick brown fox jumps over the lazy dog near the river bank").as_deref(), Some("en"));
        assert_eq!(detect_lang("Le renard brun rapide saute par-dessus le chien paresseux près de la rivière").as_deref(), Some("fr"));
        assert_eq!(detect_lang("Der schnelle braune Fuchs springt über den faulen Hund am Flussufer").as_deref(), Some("de"));
        assert_eq!(detect_lang("El rápido zorro marrón salta sobre el perro perezoso junto al río").as_deref(), Some("es"));
    }

    #[test]
    fn lang_unsure_or_empty() {
        assert_eq!(detect_lang(""), None);
        assert_eq!(detect_lang("12345 !!! ---"), None);
        assert_eq!(detect_lang("OK").as_deref(), Some("en"));
        assert_eq!(detect_lang("Glint").as_deref(), Some("en"));
    }
}
