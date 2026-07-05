use serde_json::Value;

pub struct SseDecoder {
    buf: String,
}

impl Default for SseDecoder {
    fn default() -> Self {
        SseDecoder::new()
    }
}

impl SseDecoder {
    pub fn new() -> Self {
        SseDecoder { buf: String::new() }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            self.buf.push_str(s);
        }
    }

    pub fn drain(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(idx) = self.buf.find("\n\n") {
            let frame: String = self.buf.drain(..idx + 2).collect();
            for line in frame.lines() {
                let line = line.trim_end_matches('\r');
                if let Some(rest) = line.strip_prefix("data:") {
                    let data = rest.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    out.push(data.to_string());
                }
            }
        }
        out
    }

    pub fn flush_remaining(&mut self) -> Vec<String> {
        if self.buf.trim().is_empty() {
            return Vec::new();
        }
        let frame = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                let data = rest.trim();
                if !data.is_empty() && data != "[DONE]" {
                    out.push(data.to_string());
                }
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
    fn parse_chunk_extracts_json() {
        let v = parse_chunk("{\"role\":\"assistant\",\"content\":\"hi\"}").unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["content"], "hi");
    }
}
