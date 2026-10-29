//! A small incremental Server-Sent Events parser.
//!
//! Network chunks can end anywhere: in the middle of a line, in the middle of a multi-byte
//! UTF-8 character, or between the `\r` and `\n` of a CRLF. The parser therefore works on bytes,
//! only decodes complete lines, and holds back a trailing `\r` until it knows what follows.
//!
//! Follows the WHATWG event-stream rules that matter here: lines end in `\n`, `\r\n` or `\r`;
//! `:` starts a comment; a single space after the colon is dropped; several `data:` lines of one
//! event are joined with `\n`; a blank line dispatches the event; an event without data is not
//! dispatched; a leading byte-order mark is ignored.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// The `event:` field, if the event had one.
    pub event: Option<String>,
    /// The joined `data:` lines.
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    started: bool,
    event: Option<String>,
    data: Option<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one network chunk and get back the events it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        if !self.started {
            // Wait until a possible BOM can be recognised or ruled out.
            if self.buf.len() < 3 && b"\xEF\xBB\xBF".starts_with(&self.buf) {
                return Vec::new();
            }
            self.started = true;
            if self.buf.starts_with(b"\xEF\xBB\xBF") {
                self.buf.drain(..3);
            }
        }
        let mut out = Vec::new();
        let mut consumed = 0;
        loop {
            let rest = &self.buf[consumed..];
            let Some(pos) = rest.iter().position(|&b| b == b'\n' || b == b'\r') else {
                break;
            };
            let mut next = pos + 1;
            if rest[pos] == b'\r' {
                match rest.get(pos + 1) {
                    Some(b'\n') => next += 1,
                    // A lone CR at the end of the buffer might be the first half of CRLF.
                    None => break,
                    Some(_) => {}
                }
            }
            let line = String::from_utf8_lossy(&rest[..pos]).into_owned();
            consumed += next;
            if let Some(ev) = self.line(&line) {
                out.push(ev);
            }
        }
        self.buf.drain(..consumed);
        out
    }

    /// Call when the stream has ended. Flushes a final unterminated line, and an event that was
    /// not followed by its blank line (servers sometimes close right after the last `data:`).
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let mut bytes = std::mem::take(&mut self.buf);
            while matches!(bytes.last(), Some(b'\r') | Some(b'\n')) {
                bytes.pop();
            }
            let line = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(ev) = self.line(&line) {
                out.push(ev);
            }
        }
        if let Some(ev) = self.dispatch() {
            out.push(ev);
        }
        out
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => match &mut self.data {
                Some(d) => {
                    d.push('\n');
                    d.push_str(value);
                }
                None => self.data = Some(value.to_string()),
            },
            // `id` and `retry` do not matter for a one-shot request.
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        let data = self.data.take()?;
        Some(SseEvent { event, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::new();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.feed(c));
        }
        out.extend(p.finish());
        out
    }

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent { event: event.map(str::to_owned), data: data.to_owned() }
    }

    #[test]
    fn parses_named_events() {
        let got = parse_all(&[b"event: ping\ndata: {\"a\":1}\n\nevent: x\ndata: 2\n\n"]);
        assert_eq!(got, vec![ev(Some("ping"), "{\"a\":1}"), ev(Some("x"), "2")]);
    }

    #[test]
    fn data_only_events_and_done_marker() {
        let got = parse_all(&[b"data: {\"x\":1}\n\ndata: [DONE]\n\n"]);
        assert_eq!(got, vec![ev(None, "{\"x\":1}"), ev(None, "[DONE]")]);
    }

    #[test]
    fn multiple_data_lines_join_with_newline() {
        let got = parse_all(&[b"data: a\ndata: b\ndata:\ndata: c\n\n"]);
        assert_eq!(got, vec![ev(None, "a\nb\n\nc")]);
    }

    #[test]
    fn comments_unknown_fields_and_empty_events_are_ignored() {
        let got = parse_all(&[b": keep-alive\nid: 7\nretry: 100\n\nevent: only-name\n\ndata: ok\n\n"]);
        assert_eq!(got, vec![ev(None, "ok")]);
    }

    #[test]
    fn only_one_space_after_the_colon_is_stripped() {
        let got = parse_all(&[b"data:  two spaces\n\ndata:none\n\n"]);
        assert_eq!(got, vec![ev(None, " two spaces"), ev(None, "none")]);
    }

    #[test]
    fn handles_crlf_and_lone_cr() {
        let got = parse_all(&[b"event: a\r\ndata: 1\r\n\r\ndata: 2\r\rdata: 3\n\n"]);
        assert_eq!(got, vec![ev(Some("a"), "1"), ev(None, "2"), ev(None, "3")]);
    }

    #[test]
    fn utf8_bom_is_skipped() {
        let got = parse_all(&[b"\xEF\xBB\xBFdata: hi\n\n"]);
        assert_eq!(got, vec![ev(None, "hi")]);
        // And when the BOM itself is split across chunks.
        let got = parse_all(&[b"\xEF", b"\xBB\xBFdata: hi\n\n"]);
        assert_eq!(got, vec![ev(None, "hi")]);
    }

    #[test]
    fn finish_flushes_an_unterminated_event() {
        let got = parse_all(&[b"data: last"]);
        assert_eq!(got, vec![ev(None, "last")]);
        let got = parse_all(&[b"data: a\n"]);
        assert_eq!(got, vec![ev(None, "a")]);
    }

    const SAMPLE: &str = "event: message_start\r\n\
        data: {\"type\":\"message_start\"}\r\n\r\n\
        event: content_block_delta\r\n\
        data: {\"delta\":{\"text\":\"你好，世界\"}}\r\n\r\n\
        : comment line\r\n\
        data: line one\r\n\
        data: line two\r\n\r\n\
        data: tail\n\n";

    fn expected_sample() -> Vec<SseEvent> {
        vec![
            ev(Some("message_start"), "{\"type\":\"message_start\"}"),
            ev(Some("content_block_delta"), "{\"delta\":{\"text\":\"你好，世界\"}}"),
            ev(None, "line one\nline two"),
            ev(None, "tail"),
        ]
    }

    #[test]
    fn whole_buffer_matches_expected() {
        assert_eq!(parse_all(&[SAMPLE.as_bytes()]), expected_sample());
    }

    #[test]
    fn every_two_way_split_gives_the_same_events() {
        let bytes = SAMPLE.as_bytes();
        for i in 0..=bytes.len() {
            let got = parse_all(&[&bytes[..i], &bytes[i..]]);
            assert_eq!(got, expected_sample(), "split at byte {i}");
        }
    }

    #[test]
    fn every_three_way_split_gives_the_same_events() {
        let bytes = SAMPLE.as_bytes();
        for i in 0..=bytes.len() {
            for j in i..=bytes.len() {
                let got = parse_all(&[&bytes[..i], &bytes[i..j], &bytes[j..]]);
                assert_eq!(got, expected_sample(), "splits at {i} and {j}");
            }
        }
    }

    #[test]
    fn one_byte_at_a_time_gives_the_same_events() {
        let chunks: Vec<&[u8]> = SAMPLE.as_bytes().chunks(1).collect();
        assert_eq!(parse_all(&chunks), expected_sample());
    }

    #[test]
    fn split_between_cr_and_lf_is_one_terminator() {
        // "data: a\r" | "\n\r" | "\ndata: b\r\n\r\n"
        let got = parse_all(&[b"data: a\r", b"\n\r", b"\ndata: b\r\n\r\n"]);
        assert_eq!(got, vec![ev(None, "a"), ev(None, "b")]);
    }
}
