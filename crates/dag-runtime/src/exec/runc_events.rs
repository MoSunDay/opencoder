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
    root: &Path,
    path: &Path,
    offset: &mut u64,
    store: &dyn Store,
    session: &str,
    log: Option<&super::logs::StepLog>,
) -> Result<()> {
    let mut file = match super::native::files::read(root, path) {
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
        if let Some(log) = log {
            for record in &records {
                if let Some(kind @ ("text_delta" | "reasoning_delta" | "tool_start" | "tool_end")) =
                    record.sse_kind.as_deref()
                {
                    log.session_event(kind, &record.payload);
                }
            }
        }
    }
    *offset += consumed as u64;
    Ok(())
}

pub(super) async fn import_messages(
    root: &Path,
    path: &Path,
    store: &dyn Store,
    session: &str,
) -> Result<bool> {
    let bytes = match super::native::files::read_bounded(root, path, 8 * 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(false)
        }
        Err(error) => return Err(error),
    };
    let messages: Vec<opencoder_core::Message> =
        serde_json::from_slice(&bytes).context("invalid Agent messages")?;
    store.append_messages(session, &messages).await?;
    Ok(true)
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

    #[test]
    fn reasoning_and_tool_logs_preserve_typed_payload_and_instance() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let log = super::super::logs::StepLog::new("review".into(), sender).with_instance(Some(2));
        for event in [
            SessionEvent::ReasoningDelta("inspect input".into()),
            SessionEvent::ToolStart {
                id: "call-1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command":"pwd"}),
            },
            SessionEvent::ToolEnd {
                id: "call-1".into(),
                name: "bash".into(),
                output: "/workspace/review".into(),
                is_error: false,
                images: vec![],
            },
        ] {
            log.session_event(event.sse_kind(), &event.sse_data());
            let frame = receiver.try_recv().unwrap();
            assert_eq!(frame.step.as_deref(), Some("review"));
            assert_eq!(frame.payload["event"], event.sse_kind());
            assert_eq!(frame.payload["data"], event.sse_data());
            assert_eq!(frame.payload["index"], 2);
        }
    }
}
