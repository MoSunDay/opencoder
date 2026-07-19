//! Incremental SSE frame decoder for the client. Unlike the LLM client decoder
//! (which only needs `data:` lines), this also captures the `event:` field so a
//! remote client can rebuild the granular `SessionEvent` variant from the
//! server's wire format:
//!
//! ```text
//! event: text_delta
//! data: {"text":"hello"}
//!
//! ```
//!
//! A frame is terminated by a blank line. `id:`/`retry:` fields are ignored.
//! `\r\n` and bare `\r` are normalized to `\n` so the decoder is agnostic to the
//! server's newline convention (mirrors `opencoder_llm::sse`).

pub struct SseFrameDecoder {
    buf: Vec<u8>,
}

/// One decoded SSE frame: the optional `event:` name and the `data:` payload
/// (multiple `data:` lines are joined with `\n`, per the SSE spec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

impl Default for SseFrameDecoder {
    fn default() -> Self {
        SseFrameDecoder::new()
    }
}

impl SseFrameDecoder {
    pub fn new() -> Self {
        SseFrameDecoder { buf: Vec::new() }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    pub fn drain(&mut self) -> Vec<SseFrame> {
        // Process only the valid UTF-8 prefix; retain any incomplete multi-byte
        // tail (a char split across TCP reads) for the next chunk.
        let valid_len = match std::str::from_utf8(&self.buf) {
            Ok(_) => self.buf.len(),
            Err(e) => {
                let valid = e.valid_up_to();
                if valid == 0 {
                    return Vec::new();
                }
                valid
            }
        };
        let s = std::str::from_utf8(&self.buf[..valid_len]).unwrap_or("");
        let remainder_bytes = &self.buf[valid_len..];

        let normalized: String = s.replace("\r\n", "\n").replace('\r', "\n");

        let mut out = Vec::new();
        let mut start = 0;
        while let Some(rel) = normalized[start..].find("\n\n") {
            let frame_end = start + rel + 2;
            let frame = &normalized[start..frame_end];
            if let Some(parsed) = parse_frame(frame) {
                out.push(parsed);
            }
            start = frame_end;
        }

        // Retain the unconsumed normalized tail + any incomplete UTF-8 bytes.
        let remaining = &normalized[start..];
        let mut new_buf = Vec::with_capacity(remaining.len() + remainder_bytes.len());
        new_buf.extend_from_slice(remaining.as_bytes());
        new_buf.extend_from_slice(remainder_bytes);
        self.buf = new_buf;

        out
    }

    /// Flush any buffered partial frame (no trailing blank line yet).
    pub fn flush_remaining(&mut self) -> Vec<SseFrame> {
        let s = String::from_utf8_lossy(&self.buf);
        let normalized: String = s.replace("\r\n", "\n").replace('\r', "\n");
        self.buf.clear();
        if let Some(f) = parse_frame(&normalized) {
            vec![f]
        } else {
            Vec::new()
        }
    }
}

fn parse_frame(frame: &str) -> Option<SseFrame> {
    let mut event: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();
    for line in frame.lines() {
        let line = line.strip_prefix('\u{feff}').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        // id: / retry: / comments (:) are ignored
    }
    if event.is_none() && data_lines.is_empty() {
        return None;
    }
    Some(SseFrame {
        event,
        data: data_lines.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_event_and_data() {
        let mut d = SseFrameDecoder::new();
        d.push(b"event: text_delta\ndata: {\"text\":\"hi\"}\n\n");
        let f = d.drain();
        assert_eq!(
            f,
            vec![SseFrame {
                event: Some("text_delta".into()),
                data: "{\"text\":\"hi\"}".into()
            }]
        );
    }

    #[test]
    fn joins_multiple_data_lines() {
        let mut d = SseFrameDecoder::new();
        d.push(b"event: multi\ndata: a\ndata: b\n\n");
        let f = d.drain();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].data, "a\nb");
    }

    #[test]
    fn holds_partial_until_blank_line() {
        let mut d = SseFrameDecoder::new();
        d.push(b"event: text_delta\ndata: {}");
        assert!(d.drain().is_empty());
        d.push(b"\n\n");
        assert_eq!(d.drain().len(), 1);
    }

    #[test]
    fn normalizes_crlf() {
        let mut d = SseFrameDecoder::new();
        d.push(b"event: x\r\ndata: 1\r\n\r\n");
        assert_eq!(d.drain().len(), 1);
    }

    #[test]
    fn ignores_id_and_retry_and_comments() {
        let mut d = SseFrameDecoder::new();
        d.push(b":comment\nevent: e\ndata: d\nid: 5\nretry: 1000\n\n");
        let f = d.drain();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].event.as_deref(), Some("e"));
        assert_eq!(f[0].data, "d");
    }

    #[test]
    fn flush_remaining_emits_unterminated_frame() {
        let mut d = SseFrameDecoder::new();
        d.push(b"event: e\ndata: tail");
        let f = d.flush_remaining();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].data, "tail");
    }

    #[test]
    fn empty_data_frame_is_dropped() {
        let mut d = SseFrameDecoder::new();
        d.push(b"\n\n");
        assert!(d.drain().is_empty());
    }
}
