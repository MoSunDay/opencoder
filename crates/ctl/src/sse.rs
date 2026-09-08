//! Minimal SSE client: parses `event:` / `id:` / `data:` frames off a raw
//! byte stream and prints one compact JSON line per event. Self-contained on
//! purpose — the ctl binary does not link the llm crate.

use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;

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
                        while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                            let line: Vec<u8> = buffer.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
                            if let Some(frame) = assembler.feed(&line) {
                                emit(&frame);
                            }
                        }
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
    let data: Value = match serde_json::from_str(&frame.data) {
        Ok(value) => value,
        Err(_) => Value::String(frame.data.clone()),
    };
    let seq: Value = frame
        .id
        .as_deref()
        .and_then(|id| id.parse::<i64>().ok())
        .map(Value::from)
        .unwrap_or(Value::Null);
    crate::out::json_line(&serde_json::json!({
        "event": frame.event,
        "seq": seq,
        "data": data,
    }));
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
}
