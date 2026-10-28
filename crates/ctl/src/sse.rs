//! Minimal SSE client: parses `event:` / `id:` / `data:` frames off a raw
//! byte stream and prints one compact JSON line per event. Self-contained on
//! purpose — the ctl binary does not link the llm crate.

use anyhow::Result;
use futures::StreamExt;

/// One parsed SSE frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub event: Option<String>,
    pub id: Option<String>,
    pub data: String,
}

/// Incremental line-based frame assembler: feed complete lines, receive a
/// finished frame after each blank line.
#[derive(Default)]
pub struct FrameAssembler {
    frame: Frame,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one line (without the trailing newline). Returns the finished
    /// frame when the line closes the previous event (blank line).
    pub fn feed(&mut self, line: &str) -> Option<Frame> {
        if line.is_empty() {
            if self.frame.event.is_none() && self.frame.id.is_none() && self.frame.data.is_empty() {
                return None;
            }
            return Some(std::mem::take(&mut self.frame));
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.frame.event = Some(value.to_owned()),
            "id" => self.frame.id = Some(value.to_owned()),
            "data" => {
                if !self.frame.data.is_empty() {
                    self.frame.data.push('\n');
                }
                self.frame.data.push_str(value);
            }
            _ => {} // comments (`:...`) and unknown fields ignored
        }
        None
    }
}

/// Print an SSE stream as one compact JSON line per event:
/// `{"event":..,"seq":..,"data":..}`. `data` stays raw text when it is not
/// valid JSON. Stops on stream end or Ctrl-C.
pub async fn print_stream(response: reqwest::Response) -> Result<()> {
    let status = response.status().as_u16();
    anyhow::ensure!(
        (200..300).contains(&status),
        "event stream failed: HTTP {status}"
    );
    let mut stream = response.bytes_stream();
    let mut assembler = FrameAssembler::new();
    let mut buffer: Vec<u8> = Vec::new();
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            _ = &mut interrupted => {
                crate::out::note("interrupted");
                return Ok(());
            }
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buffer.extend_from_slice(&bytes);
                        let mut consumed = 0;
                        while let Some(offset) = buffer[consumed..].iter().position(|b| *b == b'\n') {
                            let end = consumed + offset;
                            let line = String::from_utf8_lossy(&buffer[consumed..end]);
                            if let Some(frame) = assembler.feed(&line) {
                                emit(&frame);
                            }
                            consumed = end + 1;
                        }
                        buffer.drain(..consumed);
                    }
                    Some(Err(error)) => return Err(error.into()),
                    None => {
                        if !buffer.is_empty() {
                            let line = String::from_utf8_lossy(&buffer).into_owned();
                            if let Some(frame) = assembler.feed(&line) {
                                emit(&frame);
                            }
                        }
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn emit(frame: &Frame) {
    use std::io::Write;
    let line = frame_line(frame);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&line);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

/// Render one wrapper line. Valid-JSON data is validated (no parse, no
/// re-serialization) and embedded raw; anything else becomes a JSON string.
fn frame_line(frame: &Frame) -> Vec<u8> {
    let mut line: Vec<u8> = Vec::with_capacity(frame.data.len() + 48);
    line.extend_from_slice(b"{\"event\":");
    match frame.event.as_deref() {
        Some(event) => serde_json::to_writer(&mut line, event).expect("serialize event"),
        None => line.extend_from_slice(b"null"),
    }
    line.extend_from_slice(b",\"seq\":");
    match frame.id.as_deref().and_then(|id| id.parse::<i64>().ok()) {
        Some(seq) => line.extend_from_slice(seq.to_string().as_bytes()),
        None => line.extend_from_slice(b"null"),
    }
    line.extend_from_slice(b",\"data\":");
    if serde_json::from_str::<serde::de::IgnoredAny>(&frame.data).is_ok() {
        line.extend_from_slice(frame.data.as_bytes());
    } else {
        serde_json::to_writer(&mut line, frame.data.as_str()).expect("serialize data");
    }
    line.extend_from_slice(b"}");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assemble(lines: &[&str]) -> Vec<Frame> {
        let mut asm = FrameAssembler::new();
        let mut frames = Vec::new();
        for line in lines {
            if let Some(frame) = asm.feed(line) {
                frames.push(frame);
            }
        }
        frames
    }

    #[test]
    fn parses_event_id_data_frame() {
        let frames = assemble(&["event: status", "id: 42", "data: {\"a\":1}", "", ""]);
        assert_eq!(
            frames,
            vec![Frame {
                event: Some("status".into()),
                id: Some("42".into()),
                data: r#"{"a":1}"#.into(),
            }]
        );
    }

    #[test]
    fn joins_multi_line_data_and_ignores_comments() {
        let frames = assemble(&[":keepalive", "data: one", "data: two", ""]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "one\ntwo");
        assert_eq!(frames[0].event, None);
    }

    #[test]
    fn consecutive_blank_lines_do_not_emit_empty_frames() {
        assert!(assemble(&["", "", ""]).is_empty());
    }

    #[test]
    fn line_scanner_splits_frames_without_per_line_allocations() {
        // Mirrors print_stream's cursor scan over a chunk with two newlines.
        let mut assembler = FrameAssembler::new();
        let mut buffer = b"event: status\ndata: {\"a\":1}\n\ndata: 2\n".to_vec();
        let mut consumed = 0;
        let mut emitted = Vec::new();
        while let Some(offset) = buffer[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + offset;
            let line = String::from_utf8_lossy(&buffer[consumed..end]);
            if let Some(frame) = assembler.feed(&line) {
                emitted.push(frame);
            }
            consumed = end + 1;
        }
        buffer.drain(..consumed);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].data, r#"{"a":1}"#);
        // The trailing newline completes the `data: 2` line, so nothing
        // partial survives in the buffer.
        assert!(buffer.is_empty());
    }

    #[test]
    fn frame_line_passes_json_data_through_raw() {
        let frame = Frame {
            event: Some("status".into()),
            id: Some("7".into()),
            data: r#"{"a": 1,"b":[1,2]}"#.into(),
        };
        let line = String::from_utf8(frame_line(&frame)).unwrap();
        assert_eq!(line, r#"{"event":"status","seq":7,"data":{"a": 1,"b":[1,2]}}"#);
        assert!(serde_json::from_str::<serde_json::Value>(&line).is_ok());
    }

    #[test]
    fn frame_line_wraps_non_json_data_and_missing_fields() {
        let frame = Frame::default();
        let mut frame = Frame { data: "plain \"text\"".into(), ..frame };
        frame.event = None;
        frame.id = Some("not-a-number".into());
        let line = String::from_utf8(frame_line(&frame)).unwrap();
        assert_eq!(
            line,
            r#"{"event":null,"seq":null,"data":"plain \"text\""}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["data"], "plain \"text\"");
    }
}
