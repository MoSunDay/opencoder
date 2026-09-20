//! Incrementally import container Agent events into its host-side child session.
use anyhow::{Context, Result};
use opencoder_session::SessionEvent;
use opencoder_store::{SessionEventRecord, Store};
use serde_json::Value;
use std::{
    io::{Read, Seek},
    path::Path,
};

pub(super) async fn drain(
    path: &Path,
    offset: &mut u64,
    store: &dyn Store,
    session: &str,
) -> Result<()> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    file.seek(std::io::SeekFrom::Start(*offset))?;
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024).read_to_end(&mut bytes)?;
    let (records, consumed) = parse(&bytes, session)?;
    if !records.is_empty() {
        store.append_events(&records).await?;
    }
    *offset += consumed as u64;
    Ok(())
}

fn parse(bytes: &[u8], session: &str) -> Result<(Vec<SessionEventRecord>, usize)> {
    let mut records = Vec::new();
    let mut consumed = 0;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if line.last() != Some(&b'\n') {
            break;
        }
        let value: Value = serde_json::from_slice(line).context("invalid container event")?;
        let kind = value["kind"]
            .as_str()
            .context("container event kind missing")?;
        let event = SessionEvent::from_sse(kind, value["payload"].clone())
            .context("unknown container event")?;
        if !event.is_sidecar_frame() {
            records.push(SessionEventRecord {
                session_id: session.into(),
                kind: event.coarse_kind(),
                payload: event.sse_data(),
                ts: opencoder_core::message::now_ms(),
                seq: None,
                sse_kind: Some(event.sse_kind().into()),
            });
        }
        consumed += line.len();
    }
    anyhow::ensure!(
        consumed > 0 || bytes.len() < 2 * 1024 * 1024,
        "container event exceeds 2 MiB"
    );
    Ok((records, consumed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_records_keep_session_identity_and_partial_lines_wait() {
        let line = b"{\"kind\":\"text_delta\",\"payload\":{\"text\":\"hello\"}}\n";
        let mut bytes = line.to_vec();
        bytes.extend_from_slice(b"{\"kind\":");
        let (records, consumed) = parse(&bytes, "instance-session").unwrap();
        assert_eq!(consumed, line.len());
        assert_eq!(records[0].session_id, "instance-session");
        assert_eq!(records[0].payload["text"], "hello");
        assert!(parse(b"invalid\n", "s").is_err());
    }
}
