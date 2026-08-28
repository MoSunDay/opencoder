//! Opencoder session bundle — binary export/import.
//!
//! Format: `[8B magic "OPENCODR"][4B version LE][8B payload_len LE][payload]`
//! Payload is a serde_json-serialized `SessionBundle`. The whole file is a
//! custom opencoder binary (`.opencoder` extension), not a raw JSON document.
//! Recursively includes subagent sessions.

use std::io::{Read, Write};

use anyhow::{Context, Result};
use opencoder_core::Message;
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
    // Orphaned (promoted-but-unrecorded) rows are invisible to
    // `pending_inputs`; recover first so the bundle exports them instead of
    // silently dropping them. Best-effort — an export of a live session may
    // race an active drain either way.
    let _ = store.recover_orphan_inputs(session_id).await;
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
    const MAX_BUNDLE: usize = 256 * 1024 * 1024; // 256 MiB
    if len > MAX_BUNDLE {
        anyhow::bail!("bundle payload too large: {len} bytes (max {MAX_BUNDLE})");
    }
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
    // Guard against maliciously/deeply nested bundles that could overflow the
    // stack via unbounded recursion through `subagents`. 32 is far beyond any
    // legitimate nesting (real subagent trees are < 5 deep).
    const MAX_BUNDLE_DEPTH: usize = 32;
    if depth > MAX_BUNDLE_DEPTH {
        return Err(anyhow::anyhow!(
            "bundle import exceeded max recursion depth {MAX_BUNDLE_DEPTH} (cyclic nesting?)"
        ));
    }
    let session_id = bundle.meta.id.clone();

    // Skip if session already exists (idempotent).
    if store.get_session(&session_id).await?.is_some() {
        return Ok(session_id);
    }

    // Create session row with target workdir_hash.
    let mut meta = bundle.meta.clone();
    if workdir_hash.is_some() {
        meta.workdir_hash = workdir_hash.map(|h| h.to_string());
    }
    // The bundle's compaction/handoff markers reference message seqs from the
    // SOURCE database, but `append_messages` below assigns FRESH auto-increment
    // seqs to the imported messages. Carrying `summary_seq`/`handoff_seq` (and
    // their text payloads) over unchanged would make them dangle — pointing at
    // non-existent or wrong rows and corrupting compaction/handoff boundaries
    // on resume. Reset them to match the jsonl importer (`import_jsonl_file`);
    // resume recomputes compaction state as needed.
    meta.summary = None;
    meta.summary_seq = None;
    meta.summary_images = Vec::new();
    meta.handoff_seq = None;
    meta.handoff_plan = None;

    // Any failure AFTER `create_session` commits would leave an empty session
    // stub behind, and the idempotency guard above would then skip this session
    // on every retry — a permanent half-import. Wrap the whole body so that on
    // ANY error we roll the stub back (deleting only THIS session; child
    // subagent sessions are independent and self-protect via their own
    // recursion-wrapped rollback), then propagate the error. Mirrors
    // `import_jsonl_file`.
    if let Err(e) = async {
        store.create_session(&meta).await?;

        // Bulk insert messages.
        if !bundle.messages.is_empty() {
            store.append_messages(&session_id, &bundle.messages).await?;
        }

        // Insert events (single batched transaction).
        if !bundle.events.is_empty() {
            store.append_events(&bundle.events).await?;
        }

        // Insert pending inputs.
        for input in &bundle.inputs {
            store.admit_input(input).await?;
        }

        // Recursively import subagent children (child session first, then
        // link). A child failure propagates up and triggers THIS session's
        // rollback; the child cleans itself via its own recursion-wrapped
        // rollback, so we only delete `session_id` here.
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

        Ok::<_, anyhow::Error>(())
    }
    .await
    {
        let _ = store.delete_session(&session_id).await;
        return Err(e);
    }

    Ok(session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EventKind, SubagentStatus};
    use crate::LibsqlStore;
    use opencoder_core::{ContentBlock, Message, MessageUsage, Role};

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
                autopilot_mode: None,
                workdir_hash: Some("abc".into()),
                created_at: 1000,
                updated_at: 2000,
                summary: None,
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                summary_images: vec![],
                skill: None,
                task_type: None,
                requirement: None,
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

    /// A crafted bundle advertising a huge payload length must be rejected
    /// with an error (capped by MAX_BUNDLE) rather than triggering an
    /// unbounded allocation that would OOM/crash the process.
    #[test]
    fn rejects_oversized_payload() {
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        // Claim a 1 GiB payload — well above the 256 MiB cap.
        let huge = 1024 * 1024 * 1024u64;
        buf.extend_from_slice(&huge.to_le_bytes());
        // No actual payload bytes follow; the length check must fire first.
        let mut cursor = std::io::Cursor::new(&buf);
        let res = read_bundle(&mut cursor);
        assert!(res.is_err(), "oversized payload should error");
        let msg = format!("{}", res.unwrap_err());
        assert!(
            msg.contains("too large"),
            "error should mention 'too large', got: {msg}"
        );
    }

    /// C1: stale compaction/handoff markers from the source DB must be reset on
    /// import, since `append_messages` reassigns fresh auto-increment seqs (the
    /// source seqs would otherwise dangle). Mirrors the jsonl importer.
    #[tokio::test]
    async fn import_bundle_resets_summary_handoff_seq() {
        let store = LibsqlStore::open_memory().await.unwrap();

        let bundle = SessionBundle {
            meta: SessionMeta {
                id: "sess-c1".into(),
                title: Some("t".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                autopilot_mode: None,
                workdir_hash: None,
                created_at: 1,
                updated_at: 2,
                // Stale markers pointing at source-DB seqs that won't exist here.
                summary: Some("stale summary".into()),
                summary_seq: Some(3),
                summary_images: vec![],
                handoff_seq: Some(2),
                handoff_plan: Some("stale plan".into()),
                skill: None,
                task_type: None,
                requirement: None,
            },
            messages: vec![Message::user("u1", "hi"), Message::assistant("a1")],
            events: vec![],
            inputs: vec![],
            subagents: vec![],
        };

        let id = import_bundle(&store, &bundle, None).await.unwrap();
        assert_eq!(id, "sess-c1");

        let meta = store
            .get_session("sess-c1")
            .await
            .unwrap()
            .expect("session must be present after import");
        assert_eq!(
            meta.summary_seq, None,
            "summary_seq must be reset on import"
        );
        assert_eq!(
            meta.handoff_seq, None,
            "handoff_seq must be reset on import"
        );
        assert_eq!(
            meta.handoff_plan, None,
            "handoff_plan must be reset on import"
        );
        assert_eq!(meta.summary, None, "summary must be reset on import");
    }

    /// Bug 5: when `import_bundle` is called with `workdir_hash=None`, a
    /// subagent child must preserve its OWN `workdir_hash` from the bundle
    /// rather than being wiped to `None`. Previously the `depth > 0` branch
    /// of the workdir-override condition nulled children even when the caller
    /// passed no override — defeating the doc guarantee that every session
    /// row carries a workdir_hash.
    #[tokio::test]
    async fn import_bundle_preserves_child_workdir_hash_when_none() {
        let store = LibsqlStore::open_memory().await.unwrap();

        // Child bundle carries its own workdir_hash.
        let child = SessionBundle {
            meta: SessionMeta {
                id: "child-1".into(),
                title: Some("t".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                autopilot_mode: None,
                workdir_hash: Some("abc123".into()),
                created_at: 1,
                updated_at: 2,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            },
            messages: vec![],
            events: vec![],
            inputs: vec![],
            subagents: vec![],
        };

        let bundle = SessionBundle {
            meta: SessionMeta {
                id: "parent-1".into(),
                title: Some("t".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                autopilot_mode: None,
                workdir_hash: Some("parent-hash".into()),
                created_at: 1,
                updated_at: 2,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            },
            messages: vec![],
            events: vec![],
            inputs: vec![],
            subagents: vec![SubagentBundle {
                task: SubagentTaskRecord {
                    task_id: "task-1".into(),
                    parent_session_id: "parent-1".into(),
                    child_session_id: "child-1".into(),
                    parent_message_id: None,
                    agent: "explore".into(),
                    prompt: "explore the code".into(),
                    result: None,
                    status: SubagentStatus::Completed,
                    ok: None,
                    started_at: 1,
                    completed_at: None,
                },
                child,
            }],
        };

        // Import with NO workdir_hash override: the bundle's own hashes must win.
        let id = import_bundle(&store, &bundle, None).await.unwrap();
        assert_eq!(id, "parent-1");

        // Root preserves its own bundle workdir_hash (no override passed).
        let parent_meta = store
            .get_session("parent-1")
            .await
            .unwrap()
            .expect("parent must exist");
        assert_eq!(
            parent_meta.workdir_hash.as_deref(),
            Some("parent-hash"),
            "parent workdir_hash must be preserved when caller passes None"
        );

        // The child must preserve its OWN workdir_hash (not nulled by depth>0).
        let child_meta = store
            .get_session("child-1")
            .await
            .unwrap()
            .expect("child must exist");
        assert_eq!(
            child_meta.workdir_hash.as_deref(),
            Some("abc123"),
            "child workdir_hash must be preserved when caller passes None"
        );
    }

    /// C2: a failure AFTER `create_session` commits must roll the stub back so
    /// the idempotency guard does not permanently block retries.
    #[tokio::test]
    async fn import_bundle_rolls_back_on_failure() {
        let store = LibsqlStore::open_memory().await.unwrap();

        // message ids are NOT unique in this schema, so a duplicate-id collision
        // cannot trigger a failure. Instead, craft an event whose session_id
        // references a session that does NOT exist: the `session_events` FK
        // (foreign_keys=ON) rejects the INSERT, so `append_events` fails AFTER
        // `create_session` + `append_messages` have already committed.
        let bad = rollback_bundle("ghost-session");
        let err = import_bundle(&store, &bad, None).await;
        assert!(err.is_err(), "import must fail on the FK-violating event");

        // Rollback: the stub session row and its already-committed messages
        // (removed via ON DELETE CASCADE) must be gone.
        assert!(
            store.get_session("sess-rollback").await.unwrap().is_none(),
            "failed import must roll back the session row"
        );
        assert!(
            store
                .load_messages("sess-rollback")
                .await
                .unwrap()
                .is_empty(),
            "messages committed before the failure must be cascaded away"
        );

        // Retry with a corrected event session_id. Because the stub was rolled
        // back, the idempotency guard no longer skips it — the import succeeds.
        let good = rollback_bundle("sess-rollback");
        let id = import_bundle(&store, &good, None)
            .await
            .expect("retry must succeed after rollback");
        assert_eq!(id, "sess-rollback");
        assert_eq!(store.load_messages("sess-rollback").await.unwrap().len(), 2);
        assert!(store.get_session("sess-rollback").await.unwrap().is_some());
    }

    /// Bundle for the rollback test: a valid session + 2 messages + one event
    /// whose `session_id` is `event_session`. When `event_session` is a
    /// non-existent id, `append_events` fails on the FK constraint.
    fn rollback_bundle(event_session: &str) -> SessionBundle {
        SessionBundle {
            meta: SessionMeta {
                id: "sess-rollback".into(),
                title: Some("t".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                autopilot_mode: None,
                workdir_hash: None,
                created_at: 1,
                updated_at: 2,
                summary: None,
                summary_images: vec![],
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
            },
            messages: vec![Message::user("u1", "hi"), Message::assistant("a1")],
            events: vec![SessionEventRecord {
                session_id: event_session.into(),
                kind: EventKind::TextDelta,
                payload: serde_json::json!({}),
                ts: 1,
                seq: None,
                sse_kind: None,
            }],
            inputs: vec![],
            subagents: vec![],
        }
    }

    /// A bundle nested deeper than MAX_BUNDLE_DEPTH (32) must be rejected
    /// rather than recursing unboundedly (stack-overflow risk via crafted
    /// cyclic nesting). The guard fires at depth 33.
    #[tokio::test]
    async fn deeply_nested_bundle_exceeding_max_depth_is_rejected() {
        let store = LibsqlStore::open_memory().await.unwrap();
        // Build a chain 34 levels deep (root at depth 0, deepest child at
        // depth 33) by repeatedly wrapping in a single SubagentBundle.
        let mut bundle = sample_bundle();
        for i in 0..33u32 {
            let parent_id = format!("nest-{i}");
            bundle = SessionBundle {
                meta: SessionMeta {
                    id: parent_id.clone(),
                    ..sample_bundle().meta
                },
                messages: vec![],
                events: vec![],
                inputs: vec![],
                subagents: vec![SubagentBundle {
                    task: SubagentTaskRecord {
                        task_id: format!("task-{i}"),
                        parent_session_id: parent_id,
                        child_session_id: format!("nest-{}", i.wrapping_sub(1)),
                        parent_message_id: None,
                        agent: "explore".into(),
                        prompt: "go".into(),
                        result: None,
                        status: SubagentStatus::Completed,
                        ok: None,
                        started_at: 1,
                        completed_at: None,
                    },
                    child: bundle,
                }],
            };
        }
        let result = import_bundle(&store, &bundle, None).await;
        assert!(result.is_err(), "deeply nested bundle should be rejected");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("max recursion depth"),
            "error should mention 'max recursion depth', got: {msg}"
        );
    }
}
