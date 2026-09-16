//! Raw HTTP/1.1 + SSE client helpers for the fleet e2e suites.
//!
//! The root package deliberately keeps its test surface on the standard
//! library (no reqwest dev-dependency): these helpers hand-roll the same
//! Bearer-authenticated JSON requests the suites have always used, plus a
//! minimal SSE reader (`id:`/`event:`/`data:` lines) for the streaming
//! endpoints.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::fleet_proc::ProcLog;

/// One parsed SSE frame: `id: <seq>`, `event: <kind>`, `data: <json>`.
#[derive(Debug, Clone)]
pub struct SseFrame {
    pub seq: i64,
    pub event: String,
    pub data: Value,
}

/// Issue one Bearer-authenticated JSON request; returns `(status, json)`
/// (`Null` when the body is not JSON, e.g. an empty 204).
pub fn http(base: &str, method: &str, path: &str, token: &str, body: &str) -> (u16, Value) {
    let (status, text) = http_text(base, method, path, token, &[], Some(body));
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, json)
}

/// Issue one request with extra headers, returning the raw body text.
pub fn http_text(
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    extra_headers: &[(&str, &str)],
    body: Option<&str>,
) -> (u16, String) {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let body = body.unwrap_or("");
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {host}\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("connection: close\r\n\r\n");
    request.push_str(body);
    let _ = stream.write_all(request.as_bytes());
    // `set_read_timeout` surfaces as ErrorKind::WouldBlock mid-body; under
    // workspace-wide load a 10s stall is normal, so keep reading instead of
    // panicking (bounded by an overall deadline).
    let mut response = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match stream.read_to_end(&mut response) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "http read stalled: {path}");
            }
            Err(e) => panic!("http read failed on {path}: {e}"),
        }
    }
    let response = String::from_utf8(response).unwrap();
    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("malformed HTTP response on {path}: {response}"));
    let status = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, body.to_string())
}

/// Read SSE frames off `path` until the stream closes or
/// `stop_after_frames` frames have arrived, whichever comes first. `after`
/// resumes the cursor via the `last-event-id` header.
pub fn sse_read(
    base: &str,
    path: &str,
    token: &str,
    after: i64,
    stop_after_frames: Option<usize>,
) -> Vec<SseFrame> {
    let host = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nhost: {host}\r\nauthorization: Bearer {token}\r\naccept: text/event-stream\r\nconnection: close\r\n"
    );
    if after > 0 {
        request.push_str(&format!("last-event-id: {after}\r\n"));
    }
    request.push_str("\r\n");
    let _ = stream.write_all(request.as_bytes());
    let mut buffer = String::new();
    let mut frames = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if stop_after_frames.is_some_and(|count| frames.len() >= count) {
            return frames;
        }
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return frames, // server closed the stream
            Ok(count) => buffer.push_str(&String::from_utf8_lossy(&chunk[..count])),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("SSE read failed on {path}: {e}"),
        }
        while let Some(index) = buffer.find("\n\n") {
            let raw = buffer[..index].to_string();
            buffer.drain(..index + 2);
            if let Some(frame) = parse_frame(&raw) {
                frames.push(frame);
            }
        }
        assert!(Instant::now() < deadline, "SSE stream did not end: {path}");
    }
}

/// Parse one `id:`/`event:`/`data:` block into an [`SseFrame`].
fn parse_frame(raw: &str) -> Option<SseFrame> {
    let mut seq = 0;
    let mut event = String::new();
    let mut data = String::new();
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("id: ") {
            seq = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("event: ") {
            event = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data: ") {
            data.push_str(value.trim());
        }
    }
    if event.is_empty() && data.is_empty() {
        return None;
    }
    Some(SseFrame {
        seq,
        event,
        data: serde_json::from_str(&data).unwrap_or(Value::Null),
    })
}

/// Deadline-bounded poll loop (50ms) that panics with `label` plus the
/// process log tail on timeout — the standard wait shape for this suite.
pub fn wait_until<T>(log: &ProcLog, label: &str, secs: u64, probe: impl Fn() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after {secs}s waiting for {label}\n--- process log tail ---\n{}",
            log.tail(80)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
