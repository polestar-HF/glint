//! Paragraph, line-break and indentation preservation for translation engines.
//!
//! OCR produces structured text where single `\n` separates list items or compact UI labels and
//! `\n\n` separates distinct prose paragraphs or sections. This module decomposes that layout
//! before translation and either:
//!
//! * reassembles batch responses from engines that accept arrays of strings in a single HTTP
//!   request (DeepL, Google Cloud v2, Azure Translator, and Baidu's per-line response), or
//! * reconciles single-string responses (free Google/Bing web endpoints, Youdao, LLMs) when the
//!   engine collapsed or expanded blank lines, trimmed leading indentation, or inserted stray
//!   spaces around newlines.

/// One non-empty line with its leading horizontal indentation preserved separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub indent: String,
    pub text: String,
}

/// Lossless line/paragraph skeleton of a normalized source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub segments: Vec<Segment>,
    /// Exact newline run (`"\n"`, `"\n\n"`, ...) between `segments[i]` and `segments[i + 1]`.
    pub separators: Vec<String>,
}

impl Layout {
    /// Parse `raw` into non-empty lines and the exact newline separators between them.
    /// Leading and trailing blank lines are ignored; `\r\n` and lone `\r` are normalized to `\n`.
    pub fn parse(raw: &str) -> Self {
        let norm = normalize_newlines(raw);
        let mut segments = Vec::new();
        let mut separators = Vec::new();
        let mut blank_lines = 0usize;

        for line in norm.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !segments.is_empty() {
                    blank_lines += 1;
                }
                continue;
            }
            if !segments.is_empty() {
                separators.push("\n".repeat(1 + blank_lines));
                blank_lines = 0;
            }
            let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
            segments.push(Segment {
                indent: line[..indent_len].to_string(),
                text: line[indent_len..].trim_end().to_string(),
            });
        }

        Layout {
            segments,
            separators,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn is_multiline(&self) -> bool {
        self.segments.len() > 1
    }

    /// Borrow each non-empty line's text (without leading indentation) for batch APIs.
    pub fn texts(&self) -> Vec<&str> {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }

    /// Reconstruct the normalized source text with original indentation and blank lines intact.
    pub fn source_text(&self) -> String {
        let mut out = String::new();
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                out.push_str(&self.separators[i - 1]);
            }
            out.push_str(&seg.indent);
            out.push_str(&seg.text);
        }
        out
    }

    /// Join non-empty segments with single `\n` (no blank lines), for APIs like Baidu that split
    /// on `\n` and drop empty lines.
    pub fn compact_lines(&self) -> String {
        self.texts().join("\n")
    }

    /// Reassemble `translated` segments using this layout's exact indentation and separators.
    /// Returns `None` if the counts do not match.
    pub fn reassemble<S: AsRef<str>>(&self, translated: &[S]) -> Option<String> {
        if translated.len() != self.segments.len() {
            return None;
        }
        let mut out = String::new();
        for (i, (seg, tr)) in self.segments.iter().zip(translated).enumerate() {
            if i > 0 {
                out.push_str(&self.separators[i - 1]);
            }
            let cleaned = normalize_newlines(tr.as_ref());
            let body = cleaned.trim();
            if !seg.indent.is_empty() {
                out.push_str(&seg.indent);
            }
            out.push_str(body);
        }
        Some(out)
    }

    /// Reconcile a single-string translation against this source layout without discarding text.
    ///
    /// * Single-line source: normalizes CRLF and restores leading indentation if the source had any.
    /// * Multi-line source with matching non-empty line count: restores exact `\n` / `\n\n`
    ///   separators and indentation, cleaning stray spaces adjacent to newlines.
    /// * Multi-paragraph source (`\n\n`) whose translated paragraph count matches: reconciles each
    ///   paragraph block independently.
    /// * Anything else: returns the normalized translation unchanged so no content is ever lost.
    pub fn reconcile(&self, translated: &str) -> String {
        let norm = normalize_newlines(translated);
        if self.segments.is_empty() {
            return String::new();
        }
        if self.segments.len() == 1 {
            let indent = &self.segments[0].indent;
            if indent.is_empty() {
                return norm;
            }
            let body = norm.trim_matches('\n');
            if body.starts_with(indent.as_str()) {
                return body.to_string();
            }
            return format!("{indent}{}", body.trim_start_matches([' ', '\t']));
        }

        let out_lines: Vec<&str> = norm
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if let Some(aligned) = self.reassemble(&out_lines) {
            return aligned;
        }

        // Paragraph-block fallback when the source has blank-line (`\n\n`) paragraph breaks.
        let (src_blocks, block_seps) = self.split_blocks();
        if src_blocks.len() > 1 {
            let out_blocks = split_blank_blocks(&norm);
            if out_blocks.len() == src_blocks.len() {
                let mut out = String::new();
                for (i, (src_blk, out_blk)) in src_blocks.iter().zip(&out_blocks).enumerate() {
                    if i > 0 {
                        out.push_str(&block_seps[i - 1]);
                    }
                    out.push_str(&src_blk.reconcile(out_blk));
                }
                return out;
            }
        }

        norm.trim_matches('\n').to_string()
    }

    /// Split into sub-layouts at every separator containing a blank line (`\n\n` or more).
    fn split_blocks(&self) -> (Vec<Layout>, Vec<String>) {
        let mut blocks = Vec::new();
        let mut seps = Vec::new();
        let mut start = 0;
        for (i, sep) in self.separators.iter().enumerate() {
            if sep.len() >= 2 {
                blocks.push(Layout {
                    segments: self.segments[start..=i].to_vec(),
                    separators: self.separators[start..i].to_vec(),
                });
                seps.push(sep.clone());
                start = i + 1;
            }
        }
        if start < self.segments.len() {
            blocks.push(Layout {
                segments: self.segments[start..].to_vec(),
                separators: self.separators[start..].to_vec(),
            });
        }
        (blocks, seps)
    }
}

fn normalize_newlines(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Split normalized text into non-empty blocks separated by one or more blank lines.
fn split_blank_blocks(norm: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in norm.split('\n') {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                blocks.push(cur.join("\n"));
                cur.clear();
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        blocks.push(cur.join("\n"));
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paragraphs_lists_and_indentation() {
        let l = Layout::parse("\n\nAbout\nFast tool.\n\n  - item one\n  - item two\n\n");
        assert_eq!(
            l.texts(),
            ["About", "Fast tool.", "- item one", "- item two"]
        );
        assert_eq!(l.separators, ["\n", "\n\n", "\n"]);
        assert_eq!(l.segments[2].indent, "  ");
        assert_eq!(l.segments[3].indent, "  ");
        assert_eq!(
            l.source_text(),
            "About\nFast tool.\n\n  - item one\n  - item two"
        );
        assert_eq!(
            l.compact_lines(),
            "About\nFast tool.\n- item one\n- item two"
        );
    }

    #[test]
    fn reassembles_batch_translations_with_exact_separators_and_indent() {
        let l = Layout::parse("First paragraph.\n\n  • Item one\n  • Item two");
        let out = l
            .reassemble(&["第一段。", "• 第一项", " • 第二项 "])
            .expect("counts match");
        assert_eq!(out, "第一段。\n\n  • 第一项\n  • 第二项");
        assert_eq!(l.reassemble(&["only one"]), None);
    }

    #[test]
    fn reconcile_restores_collapsed_blank_lines_and_strips_extra_ones() {
        let para = Layout::parse("First paragraph.\n\nSecond paragraph.");
        // Engine collapsed `\n\n` into `\n` and left spaces around the newline.
        assert_eq!(
            para.reconcile("第一段。 \n 第二段。"),
            "第一段。\n\n第二段。"
        );

        let list = Layout::parse("Readme\nMIT license\nActivity");
        // Engine inserted unwanted blank lines between list items.
        assert_eq!(
            list.reconcile("自述\n\nMIT 许可证\n\n活动"),
            "自述\nMIT 许可证\n活动"
        );
    }

    #[test]
    fn reconcile_restores_indentation_and_handles_crlf() {
        let l = Layout::parse("  - one\r\n  - two");
        assert_eq!(l.reconcile("- 一\r\n- 二"), "  - 一\n  - 二");

        let single = Layout::parse("    indented");
        assert_eq!(single.reconcile("缩进"), "    缩进");
    }

    #[test]
    fn reconcile_uses_paragraph_block_fallback_when_line_count_differs() {
        let src = Layout::parse("Heading\nOne long paragraph.\n\nItem A\nItem B");
        // Block 1 was translated onto 3 lines instead of 2, block 2 kept 2 lines.
        let tr = "标题\n很长的一段。\n补充句。\n\n项目 A\n\n项目 B";
        // Notice: `out_blocks` splits on blank lines -> 3 blocks, not 2; test a 2-block case:
        let tr2 = "标题\n很长的一段。\n补充句。\n\n项目 A\n项目 B";
        assert_eq!(
            src.reconcile(tr2),
            "标题\n很长的一段。\n补充句。\n\n项目 A\n项目 B"
        );
        // When neither line count nor block count matches, text is preserved without loss.
        assert_eq!(src.reconcile(tr), tr);
    }

    #[test]
    fn single_line_without_indent_passes_through_unchanged() {
        let l = Layout::parse("Hello, world.");
        assert_eq!(l.reconcile("你好，世界。"), "你好，世界。");
        assert_eq!(l.reconcile("第一句\r\n第二句"), "第一句\n第二句");
    }
}
