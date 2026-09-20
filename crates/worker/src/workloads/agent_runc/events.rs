//! Event tailing for runc agent sessions: the runner appends Say frames
//! as ndjson lines to `events.ndjson` inside the container's run root and
//! the host relays them into the store. Kept apart from the round
//! orchestration so the parsing rules stay reviewable on their own.

use anyhow::Result;
use opencoder_session::SessionEvent;
use opencoder_store::{SessionEventRecord, Store};
use serde_json::Value;
use std::io::{Read as _, Seek as _};
use std::path::Path;

/// Tail one poll of `events.ndjson` into the store: parse every COMPLETE
/// line after `offset`, persist the records as one batch and return the
/// new offset (unchanged when nothing complete arrived). A missing file
/// (runner not started yet) is not an error.
pub(super) async fn drain_events(
    path: &Path,
    offset: u64,
    store: &dyn Store,
    session_id: &str,
    error_event: &mut Option<String>,
) -> Result<u64> {
    let bytes = match read_from(path, offset) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(offset),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(offset),
        Err(error) => return Err(error.into()),
    };
    let (records, error, consumed) = parse_event_lines(&bytes, session_id);
    if let Some(error) = error {
        *error_event = Some(error);
    }
    if !records.is_empty() {
        store.append_events(&records).await?;
    }
    Ok(offset + consumed as u64)
}

/// Read the bytes of `path` starting at `offset`; `None` when nothing new.
fn read_from(path: &Path, offset: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len <= offset {
        return Ok(None);
    }
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity((len - offset) as usize);
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Parse complete ndjson event lines (`{"kind":...,"payload":...}` in the
/// SSE wire shape) from `bytes`. Returns the store records, the LAST error
/// payload and the number of bytes consumed: a trailing INCOMPLETE line is
/// held back for the next poll, malformed or unknown lines are skipped but
/// still consumed, and sidecar frames are dropped like the host sink does
/// (their content must never reach the DB).
pub(super) fn parse_event_lines(
    bytes: &[u8],
    session_id: &str,
) -> (Vec<SessionEventRecord>, Option<String>, usize) {
    let mut records = Vec::new();
    let mut error = None;
    let mut consumed = 0usize;
    let mut rest = bytes;
    while let Some(index) = rest.iter().position(|&byte| byte == b'\n') {
        let (line, tail) = rest.split_at(index);
        rest = &tail[1..];
        consumed += index + 1;
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let (Some(kind), payload) = (
            value.get("kind").and_then(Value::as_str),
            value.get("payload").cloned().unwrap_or(Value::Null),
        ) else {
            continue;
        };
        if kind == "error" {
            error = Some(
                payload
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| payload.to_string()),
            );
        }
        let Some(event) = SessionEvent::from_sse(kind, payload) else {
            continue;
        };
        if event.is_sidecar_frame() {
            continue;
        }
        records.push(SessionEventRecord {
            session_id: session_id.to_string(),
            kind: event.coarse_kind(),
            payload: event.sse_data(),
            ts: opencoder_core::message::now_ms(),
            seq: None,
            sse_kind: Some(event.sse_kind().to_string()),
        });
    }
    (records, error, consumed)
}
