use serde_json::Value;

pub struct SseDecoder {
    buf: Vec<u8>,
}

impl Default for SseDecoder {
    fn default() -> Self {
        SseDecoder::new()
    }
}

impl SseDecoder {
    pub fn new() -> Self {
        SseDecoder { buf: Vec::new() }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    pub fn drain(&mut self) -> Vec<String> {
        // Strip leading invalid UTF-8 bytes so decoding advances instead of
        // stalling forever (Bug 7). A genuinely invalid byte
        // (`error_len == Some`) is dropped; an incomplete multi-byte *trailing*
        // sequence (`error_len == None`) is retained to await more bytes.
        //
        // The mutation (`self.buf.drain`) runs in its own statement AFTER the
        // borrow from `std::str::from_utf8(&self.buf)` is released — mutating
        // inside the match arm would conflict with the scrutinee's shared
        // borrow of `self.buf`.
        loop {
            let drop = match std::str::from_utf8(&self.buf) {
                Ok(_) => break,
                Err(e) if e.valid_up_to() == 0 => match e.error_len() {
                    Some(n) => n,
                    None => break,
                },
                Err(_) => break,
            };
            self.buf.drain(..drop.min(self.buf.len()));
        }

        // Decode as much valid UTF-8 as possible. If the tail is an
        // incomplete multi-byte sequence (a char split across TCP reads),
        // process only the valid prefix and retain the partial bytes. Frame
        // scanning is bounded by `valid_len` so terminators are never matched
        // against partial multi-byte bytes.
        let valid_len = match std::str::from_utf8(&self.buf) {
            Ok(_) => self.buf.len(),
            Err(e) => {
                let valid = e.valid_up_to();
                if valid == 0 {
                    // Pure incomplete trailing sequence (all leading invalid
                    // bytes were stripped above) — retain and wait for bytes.
                    return Vec::new();
                }
                valid
            }
        };

        // Split frames on the RAW buffer (Bug 8): normalizing CR up front
        // would turn a `\r` that pairs with the NEXT chunk's `\n` into a
        // premature frame end, splitting one event in two. CR normalization
        // is deferred to `parse_data_frame`, after the split.
        let mut out = Vec::new();
        let mut cursor = 0;
        while let Some((start, len)) = find_frame_terminator(&self.buf[cursor..valid_len]) {
            let frame_end = cursor + start + len;
            out.extend(parse_data_frame(&self.buf[cursor..frame_end]));
            cursor = frame_end;
        }

        // Retain the unconsumed RAW bytes (never normalized text), including
        // any incomplete UTF-8 tail past `valid_len`, so a trailing `\r` can
        // still pair with the next chunk's `\n`.
        if cursor > 0 {
            self.buf.drain(..cursor);
        }

        out
    }

    pub fn flush_remaining(&mut self) -> Vec<String> {
        let s = String::from_utf8_lossy(&self.buf);
        let normalized: String = s.replace("\r\n", "\n").replace('\r', "\n");
        self.buf.clear();
        let mut out = Vec::new();
        // The flushed buffer is a single (terminator-less) frame, so per the
        // SSE spec multiple `data:` fields are concatenated with `\n` into one
        // event.
        let mut data_parts: Vec<&str> = Vec::new();
        for line in normalized.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data_parts.push(rest.trim());
            }
        }
        if !data_parts.is_empty() {
            let joined = data_parts.join("\n");
            if !joined.is_empty() && joined != "[DONE]" {
                out.push(joined);
            }
        }
        out
    }
}

/// SSE frame terminators, longest first. A line ends with `\r\n`, `\n` or
/// `\r`, and an empty line ends the event — so a frame boundary is one line
/// ending immediately followed by another: `\r\n\r\n`, `\n\r\n` (`\n`-ended
/// line + `\r\n`-ended empty line), `\n\n`, `\r\r`, `\n\r` (`\n`-ended line +
/// bare-`\r`-ended empty line). Longest-first order keeps `\r\n\r\n` from
/// degrading to `\n\r\n`/`\n\r` and `\n\r\n` from degrading to `\n\r` when
/// several terminators start at the same byte.
const FRAME_TERMINATORS: [&[u8]; 5] = [b"\r\n\r\n", b"\n\r\n", b"\n\n", b"\r\r", b"\n\r"];

/// Earliest frame terminator in `bytes` as `(offset, len)`; on equal offsets
/// the longer terminator (seen first, per longest-first order) wins.
fn find_frame_terminator(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for term in FRAME_TERMINATORS {
        if let Some(pos) = find_sub(bytes, term) {
            match best {
                // A strictly earlier offset always wins; an equal one keeps
                // the already-held longer match.
                Some((held, _)) if held <= pos => {}
                _ => best = Some((pos, term.len())),
            }
        }
    }
    best
}

/// First offset of `needle` in `haystack` (both non-empty).
fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Normalize ONE raw frame's line endings and extract its `data:` payload:
/// consecutive `data:` fields join with `\n` into a single event; frames
/// without data lines (comments, `event:`/`id:` only) yield nothing and the
/// `[DONE]` sentinel is dropped. CR normalization lives here — after the
/// raw-buffer frame split — never on the shared buffer.
fn parse_data_frame(raw: &[u8]) -> Vec<String> {
    let normalized = String::from_utf8_lossy(raw)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let data_parts: Vec<&str> = normalized
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|rest| rest.trim())
        .collect();
    if data_parts.is_empty() {
        return Vec::new();
    }
    let joined = data_parts.join("\n");
    if joined.is_empty() || joined == "[DONE]" {
        Vec::new()
    } else {
        vec![joined]
    }
}

pub fn parse_chunk(data: &str) -> Option<Value> {
    serde_json::from_str::<Value>(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_splits_on_double_newline() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:{\"a\":1}\n\ndata:{\"a\":2}\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["{\"a\":1}", "{\"a\":2}"]);
    }

    #[test]
    fn drain_skips_done_marker() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:{\"a\":1}\n\ndata:[DONE]\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["{\"a\":1}"]);
    }

    #[test]
    fn drain_trims_crlf_line_endings() {
        let mut dec = SseDecoder::new();
        // \r before \n\n separator — the \r must not break frame detection
        dec.push(b"data:{\"a\":1}\r\n\ndata:{\"a\":2}\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["{\"a\":1}", "{\"a\":2}"]);
    }

    #[test]
    fn drain_concatenates_multi_line_data_per_spec() {
        // Per the SSE spec, multiple `data:` fields within ONE event frame are
        // joined with `\n` and dispatched as a single event (not separate
        // outputs).
        let mut dec = SseDecoder::new();
        dec.push(b"data:line1\ndata:line2\ndata:line3\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["line1\nline2\nline3"]);
    }

    #[test]
    fn drain_concatenates_multi_line_data_across_frames() {
        // Each frame is independent: a frame with multiple data lines yields
        // one joined event; a frame with one data line yields one event.
        let mut dec = SseDecoder::new();
        dec.push(b"data:a\ndata:b\n\ndata:c\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["a\nb", "c"]);
    }

    #[test]
    fn drain_concatenation_with_space_after_colon() {
        // The single leading space after `data:` is stripped per spec; the
        // join uses `\n`.
        let mut dec = SseDecoder::new();
        dec.push(b"data: hello\ndata: world\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["hello\nworld"]);
    }

    #[test]
    fn drain_holds_partial_until_complete() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:{\"a\":1}");
        assert!(dec.drain().is_empty());
        dec.push(b"\n\n");
        assert_eq!(dec.drain(), vec!["{\"a\":1}"]);
    }

    #[test]
    fn flush_remaining_emits_without_terminator() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:{\"a\":99}");
        assert!(dec.drain().is_empty());
        let out = dec.flush_remaining();
        assert_eq!(out, vec!["{\"a\":99}"]);
    }

    #[test]
    fn flush_remaining_concatenates_multi_line_data() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:a\ndata:b\ndata:c");
        let out = dec.flush_remaining();
        assert_eq!(out, vec!["a\nb\nc"]);
    }

    #[test]
    fn parse_chunk_extracts_json() {
        let v = parse_chunk("{\"role\":\"assistant\",\"content\":\"hi\"}").unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "hi");
    }

    #[test]
    fn drain_handles_split_utf8_across_chunks() {
        let mut dec = SseDecoder::new();
        // "héllo" = h + \xc3\xa9 + llo, split the é across two pushes
        dec.push(b"data:h\xc3");
        assert!(dec.drain().is_empty()); // incomplete char, wait
        dec.push(b"\xa9llo\n\n");
        assert_eq!(dec.drain(), vec!["héllo"]);
    }

    #[test]
    fn drain_frames_on_crlf_crlf_separator() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:{\"a\":1}\r\n\r\ndata:{\"a\":2}\r\n\r\n");
        let out = dec.drain();
        assert_eq!(out, vec!["{\"a\":1}", "{\"a\":2}"]);
    }

    #[test]
    fn drain_handles_entirely_incomplete_utf8_chunk() {
        let mut dec = SseDecoder::new();
        // A single continuation byte — not valid UTF-8 on its own
        dec.push(b"\xc3");
        assert!(
            dec.drain().is_empty(),
            "incomplete UTF-8 should yield no frames"
        );
        // Now complete it
        dec.push(b"\xa9\n\n");
        // \xc3\xa9 = é, but there's no data: prefix so nothing is extracted —
        // verify the buffer doesn't panic or corrupt
        assert!(dec.drain().is_empty());
    }

    #[test]
    fn drain_skips_invalid_leading_byte() {
        // Regression: an invalid byte at the head of the buffer used to make
        // `drain` return an empty `Vec` WITHOUT advancing, stalling the
        // decoder so every subsequent valid SSE frame was lost. The decoder
        // must instead drop the offending byte span and recover the valid
        // frame that follows.
        let mut dec = SseDecoder::new();
        // 0xFF is never a valid UTF-8 leading/continuation byte; it must be
        // skipped, leaving the valid `data: hello\n\n` frame to be decoded.
        dec.push(b"\xffdata: hello\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["hello"], "valid frame after bad byte must decode");
        // The bad byte is consumed (not retained), so the buffer is empty.
        assert!(dec.drain().is_empty(), "no stale bytes should remain");
    }

    #[test]
    fn drain_skips_run_of_invalid_leading_bytes() {
        // A contiguous run of invalid bytes at the head must all be skipped
        // before the valid frame decodes.
        let mut dec = SseDecoder::new();
        dec.push(b"\xff\xfe\xfddata: ok\n\n");
        assert_eq!(dec.drain(), vec!["ok"]);
        assert!(dec.drain().is_empty());
    }

    #[test]
    fn drain_handles_mixed_crlf_and_lf_separators() {
        let mut dec = SseDecoder::new();
        dec.push(b"data:{\"a\":1}\r\n\r\ndata:{\"a\":2}\n\ndata:{\"a\":3}\r\n\r\n");
        let out = dec.drain();
        assert_eq!(out, vec!["{\"a\":1}", "{\"a\":2}", "{\"a\":3}"]);
    }

    #[test]
    fn drain_handles_split_at_frame_boundary() {
        let mut dec = SseDecoder::new();
        // Split right after the first \n of the \n\n separator
        dec.push(b"data:{\"a\":1}\n");
        assert!(dec.drain().is_empty());
        dec.push(b"\ndata:{\"a\":2}\n\n");
        let out = dec.drain();
        assert_eq!(out, vec!["{\"a\":1}", "{\"a\":2}"]);
    }

    #[test]
    fn drain_keeps_cr_pending_across_chunks() {
        // Bug 8: a chunk ending in `\r` must stay raw until the next chunk
        // arrives — the `\r` pairs with the following `\n` as ONE line
        // ending, so both data lines belong to a single event.
        let mut dec = SseDecoder::new();
        dec.push(b"data:A\r");
        assert!(
            dec.drain().is_empty(),
            "lone trailing CR must hold the frame"
        );
        dec.push(b"\ndata:B\r\n\r\n");
        assert_eq!(dec.drain(), vec!["A\nB"]);
    }

    #[test]
    fn drain_splits_on_bare_cr_boundary_across_chunks() {
        // A bare `\r` ends a line per spec, so `\r\r` is a frame boundary
        // even when the two CRs arrive in different chunks.
        let mut dec = SseDecoder::new();
        dec.push(b"data:A\r");
        assert!(dec.drain().is_empty());
        dec.push(b"\rdata:B\r\r");
        assert_eq!(dec.drain(), vec!["A", "B"]);
    }

    #[test]
    fn drain_splits_on_lf_line_plus_cr_empty_line_across_chunks() {
        // The `\n` ending "data:A" and the `\r` ending the empty line arrive
        // in different chunks; per the SSE line-ending rules that pair is a
        // frame boundary, so the events stay separate instead of merging.
        let mut dec = SseDecoder::new();
        dec.push(b"data:A\n");
        assert!(dec.drain().is_empty());
        dec.push(b"\rdata:B\n\n");
        assert_eq!(dec.drain(), vec!["A", "B"]);
    }

    #[test]
    fn drain_prefers_longest_terminator_at_same_offset() {
        // `\r\n\r\n` must not be split into `\n\r\n` at offset+1: the frame
        // ends once, after the full four-byte terminator.
        let mut dec = SseDecoder::new();
        dec.push(b"data:A\r\n\r\ndata:B\n\n");
        assert_eq!(dec.drain(), vec!["A", "B"]);
    }
}
