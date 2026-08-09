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
        // process only the valid prefix and retain the partial bytes.
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
        let s = std::str::from_utf8(&self.buf[..valid_len]).unwrap_or("");
        let remainder_bytes = &self.buf[valid_len..];

        // Normalize \r\n and bare \r to \n so frame detection works
        // regardless of the server's newline convention.
        let normalized: String = s.replace("\r\n", "\n").replace('\r', "\n");

        let mut out = Vec::new();
        let mut start = 0;
        while let Some(rel) = normalized[start..].find("\n\n") {
            let frame_end = start + rel + 2;
            let frame = &normalized[start..frame_end];
            // Per the SSE spec, consecutive `data:` fields within one event
            // frame are concatenated with `\n` and dispatched as a single
            // event — NOT emitted as separate strings.
            let mut data_parts: Vec<&str> = Vec::new();
            for line in frame.lines() {
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
            start = frame_end;
        }

        // Rebuild the buffer: unconsumed normalized tail + any incomplete
        // UTF-8 bytes from the original buffer.
        let remaining = &normalized[start..];
        let mut new_buf = Vec::with_capacity(remaining.len() + remainder_bytes.len());
        new_buf.extend_from_slice(remaining.as_bytes());
        new_buf.extend_from_slice(remainder_bytes);
        self.buf = new_buf;

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
}
