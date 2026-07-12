//! Opencoder session bundle — binary export/import.
//!
//! Format: `[8B magic "OPENCODR"][4B version LE][8B payload_len LE][payload]`
//! Payload is a serde_json-serialized `SessionBundle`. The whole file is a
//! custom opencoder binary (`.opencoder` extension), not a raw JSON document.
//! Recursively includes subagent sessions.

use std::io::{Read, Write};

use anyhow::{Context, Result};
use opencode_core::Message;
use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::types::{Delivery, SessionEventRecord, SessionInput, SessionMeta, SubagentTaskRecord};

const MAGIC: &[u8; 8] = b"OPENCODR";
const FORMAT_VERSION: u32 = 1;

/// One session's full data for export/import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionBundle {
    pub meta: SessionMeta,
    pub messages: Vec<Message>,
    pub events: Vec<SessionEventRecord>,
    pub inputs: Vec<SessionInput>,
    #[serde(default)]
    pub subagents: Vec<SubagentBundle>,
}

/// A subagent task + its child session data (recursive).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentBundle {
    pub task: SubagentTaskRecord,
    pub child: SessionBundle,
}

/// Recursively collect a session and all its subagent children into a bundle.
pub async fn export_bundle(store: &dyn Store, session_id: &str) -> Result<SessionBundle> {
    let meta = store
        .get_session(session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;

    let messages = store.load_messages(session_id).await?;
    let events = store.events_after(session_id, 0).await?;
    let steer_inputs = store.pending_inputs(session_id, Delivery::Steer).await?;
    let queue_inputs = store.pending_inputs(session_id, Delivery::Queue).await?;
    let mut inputs = steer_inputs;
    inputs.extend(queue_inputs);

    let tasks = store.list_subagent_tasks(session_id).await?;
    let mut subagents = Vec::with_capacity(tasks.len());
    for task in tasks {
        let child = Box::pin(export_bundle(store, &task.child_session_id)).await;
        match child {
            Ok(bundle) => subagents.push(SubagentBundle {
                task,
                child: bundle,
            }),
            Err(e) => {
                tracing::warn!(task_id = %task.task_id, error = %e, "skipping subagent export");
            }
        }
    }

    Ok(SessionBundle {
        meta,
        messages,
        events,
        inputs,
        subagents,
    })
}

/// Write a bundle to a writer in opencoder binary format.
pub fn write_bundle(bundle: &SessionBundle, writer: &mut impl Write) -> Result<()> {
    writer.write_all(MAGIC).context("write magic")?;
    writer
        .write_all(&FORMAT_VERSION.to_le_bytes())
        .context("write version")?;
    let payload = serde_json::to_vec(bundle).context("serialize bundle")?;
    writer
        .write_all(&(payload.len() as u64).to_le_bytes())
        .context("write length")?;
    writer.write_all(&payload).context("write payload")?;
    Ok(())
}

/// Read a bundle from a reader in opencoder binary format.
pub fn read_bundle(reader: &mut impl Read) -> Result<SessionBundle> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).context("read magic")?;
    if &magic != MAGIC {
        anyhow::bail!("not an opencoder bundle (bad magic)");
    }
    let mut vbuf = [0u8; 4];
    reader.read_exact(&mut vbuf).context("read version")?;
    let version = u32::from_le_bytes(vbuf);
    if version != FORMAT_VERSION {
        anyhow::bail!("unsupported bundle version {version}");
    }
    let mut lbuf = [0u8; 8];
    reader.read_exact(&mut lbuf).context("read length")?;
    let len = u64::from_le_bytes(lbuf) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).context("read payload")?;
    serde_json::from_slice(&payload).context("deserialize bundle")
}

/// Recursively import a bundle into the store. `workdir_hash` is set on every
/// session row so they are visible in `session list` for the target workdir.
/// Returns the root session id.
pub async fn import_bundle(
    store: &dyn Store,
    bundle: &SessionBundle,
    workdir_hash: Option<&str>,
) -> Result<String> {
    import_bundle_inner(store, bundle, workdir_hash, 0).await
}

async fn import_bundle_inner(
    store: &dyn Store,
    bundle: &SessionBundle,
    workdir_hash: Option<&str>,
    depth: usize,
) -> Result<String> {
    let session_id = bundle.meta.id.clone();

    // Skip if session already exists (idempotent).
    if store.get_session(&session_id).await?.is_some() {
        return Ok(session_id);
    }

    // Create session row with target workdir_hash.
    let mut meta = bundle.meta.clone();
    if depth > 0 || workdir_hash.is_some() {
        meta.workdir_hash = workdir_hash.map(|h| h.to_string());
    }
    store.create_session(&meta).await?;

    // Bulk insert messages.
    if !bundle.messages.is_empty() {
        store.append_messages(&session_id, &bundle.messages).await?;
    }

    // Insert events.
    for ev in &bundle.events {
        store.append_event(ev).await?;
    }

    // Insert pending inputs.
    for input in &bundle.inputs {
        store.admit_input(input).await?;
    }

    // Recursively import subagent children (child session first, then link).
    for sub in &bundle.subagents {
        Box::pin(import_bundle_inner(
            store,
            &sub.child,
            workdir_hash,
            depth + 1,
        ))
        .await?;
        store.create_subagent_task(&sub.task).await?;
    }

    Ok(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencode_core::{ContentBlock, Message, MessageUsage, Role};

    fn sample_bundle() -> SessionBundle {
        let msg = Message {
            id: "msg1".into(),
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            model: Some("test-model".into()),
            agent: Some("act".into()),
            usage: MessageUsage::default(),
            created_at: 1000,
            synthetic: false,
        };
        SessionBundle {
            meta: SessionMeta {
                id: "sess1".into(),
                title: Some("test".into()),
                agent: Some("act".into()),
                model: Some("test-model".into()),
                workdir_hash: Some("abc".into()),
                created_at: 1000,
                updated_at: 2000,
                summary: None,
                summary_seq: None,
            },
            messages: vec![msg],
            events: vec![],
            inputs: vec![],
            subagents: vec![],
        }
    }

    #[test]
    fn round_trip_binary() {
        let bundle = sample_bundle();
        let mut buf = Vec::new();
        write_bundle(&bundle, &mut buf).unwrap();

        // Verify magic header.
        assert_eq!(&buf[..8], MAGIC);

        let mut cursor = std::io::Cursor::new(&buf);
        let restored = read_bundle(&mut cursor).unwrap();
        assert_eq!(restored.meta.id, "sess1");
        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0].text(), "hello");
    }

    #[test]
    fn rejects_bad_magic() {
        let bad = b"WRONGMAG\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let mut cursor = std::io::Cursor::new(&bad[..]);
        assert!(read_bundle(&mut cursor).is_err());
    }

    #[test]
    fn rejects_wrong_version() {
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&99u32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        let mut cursor = std::io::Cursor::new(&buf);
        assert!(read_bundle(&mut cursor).is_err());
    }
}
