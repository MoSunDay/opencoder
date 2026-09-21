//! Unit tests for the pure sandbox-selection and event-tail pieces; the
//! container round itself is covered by the root-package e2e suite
//! (`tests/dag_e2e/agent_runc.rs`), which skips its run section when runc
//! or a provisioned rootfs is unavailable.

use super::events::{drain_events, parse_event_lines};
use super::*;
use opencoder_core::fleet::{Assignment, CreateExecution, ExecutionIndex};
use opencoder_store::{EventKind, LibsqlStore, SessionMeta, Store};

/// Write `<root>/<name>/meta.json` with `meta`.
fn pool_card(root: &Path, name: &str, meta: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta.json"), meta).unwrap();
}

/// A minimal `kind=agent` journal record.
fn agent_record(target: Option<&str>, kind: ExecutionKind) -> Record {
    Record {
        annotations: Value::Null,
        queue: None,
        assignment: Assignment {
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: "sandbox-1".into(),
                kind,
                node_id: "node".into(),
                created_at: 1,
                status: ExecutionStatus::Idle,
            },
            request: CreateExecution {
                id: "sandbox-1".into(),
                kind,
                target: target.map(str::to_owned),
                node_id: None,
                input: json!({}),
            },
            definition: None,
        },
        result: Value::Null,
        error: None,
        events: vec![],
        lifecycle: Default::default(),
    }
}

#[test]
fn only_run_mode_agent_cards_select_the_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    pool_card(dir.path(), "myagent", r#"{"run_mode":"agent"}"#);
    pool_card(dir.path(), "hostagent", r#"{"run_mode":"operator"}"#);
    // A corrupt run_mode fails the whole meta parse; a card-less dir and a
    // missing card degrade to None — none of these may flip the sandbox.
    pool_card(dir.path(), "broken", r#"{"run_mode":"nonsense"}"#);
    std::fs::create_dir_all(dir.path().join("no-card")).unwrap();
    // Builtin names always resolve to the host registry, card or not.
    pool_card(dir.path(), "act", r#"{"run_mode":"agent"}"#);
    assert!(session_uses_sandbox(dir.path(), "myagent"));
    assert!(!session_uses_sandbox(dir.path(), "hostagent"));
    assert!(!session_uses_sandbox(dir.path(), "broken"));
    assert!(!session_uses_sandbox(dir.path(), "no-card"));
    assert!(!session_uses_sandbox(dir.path(), "missing"));
    assert!(!session_uses_sandbox(dir.path(), "act"));
}

#[test]
fn sandbox_session_requires_agent_kind_and_a_pool_scope() {
    let dir = tempfile::tempdir().unwrap();
    pool_card(dir.path(), "myagent", r#"{"run_mode":"agent"}"#);
    let record = agent_record(Some("myagent"), ExecutionKind::Agent);
    assert!(sandbox_session(&record, Some(dir.path())));
    // No snapshot on disk (fresh host, no pinned pool) keeps the native
    // path — there is no card to consult.
    assert!(!sandbox_session(&record, None));
    assert!(!sandbox_session(&record, Some(&dir.path().join("nowhere"))));
    // Operator/maintenance/members never take the sandbox path, card or not.
    for kind in [
        ExecutionKind::Operator,
        ExecutionKind::Maintenance,
        ExecutionKind::Team,
    ] {
        let record = agent_record(Some("myagent"), kind);
        assert!(!sandbox_session(&record, Some(dir.path())));
    }
    // An absent target defaults to the builtin `act` — host runtime.
    let record = agent_record(None, ExecutionKind::Agent);
    assert!(!sandbox_session(&record, Some(dir.path())));
}

#[test]
fn parse_event_lines_reconstructs_records_and_holds_partial_tail() {
    let complete = concat!(
        "{\"kind\":\"text_delta\",\"payload\":{\"text\":\"你好\"}}\n",
        "{\"kind\":\"llm_round_end\",\"payload\":{}}\n",
    );
    let partial = "{\"kind\":\"text_delta\",\"payload\":{\"text\":\"tail";
    let bytes = format!("{complete}{partial}");
    let (records, error, consumed) = parse_event_lines(bytes.as_bytes(), "sess");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].session_id, "sess");
    assert_eq!(records[0].sse_kind.as_deref(), Some("text_delta"));
    assert_eq!(records[0].kind, EventKind::TextDelta);
    assert_eq!(records[0].payload, json!({"text": "你好"}));
    assert_eq!(records[1].sse_kind.as_deref(), Some("llm_round_end"));
    assert!(error.is_none());
    // Only the complete lines count towards the offset; the partial line
    // stays unread for the next poll.
    assert_eq!(consumed, complete.len());
    // The next poll re-reads from the last complete-line boundary, so the
    // runner finishing the line yields the WHOLE line again.
    let suffix = "\"}}\n";
    let full = format!("{partial}{suffix}");
    let (records, _, consumed) = parse_event_lines(full.as_bytes(), "sess");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].payload, json!({"text": "tail"}));
    assert_eq!(consumed, full.len());
}

#[test]
fn parse_event_lines_skips_bad_lines_and_captures_error() {
    let bytes = concat!(
        "not json at all\n",
        "\n",
        "{\"kind\":\"warp_speed\",\"payload\":{}}\n",
        "{\"payload\":{}}\n",
        "{\"kind\":\"error\",\"payload\":{\"error\":\"boom\"}}\n",
        "{\"kind\":\"error\",\"payload\":{\"error\":\"later wins\"}}\n",
    );
    let (records, error, consumed) = parse_event_lines(bytes.as_bytes(), "sess");
    // Malformed JSON, blank lines, unknown kinds and kind-less objects are
    // skipped but still consumed; the last error payload wins.
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].sse_kind.as_deref(), Some("error"));
    assert_eq!(records[0].payload, json!({"error": "boom"}));
    assert_eq!(error.as_deref(), Some("later wins"));
    assert_eq!(consumed, bytes.len());
}

#[test]
fn parse_event_lines_drops_sidecar_frames() {
    let bytes = concat!(
        "{\"kind\":\"sidecar_start\",\"payload\":{\"id\":\"sc1\",\"question\":\"why\"}}\n",
        "{\"kind\":\"done\",\"payload\":{}}\n",
    );
    let (records, _, consumed) = parse_event_lines(bytes.as_bytes(), "sess");
    // Sidecar frames never reach the DB (the host sink's rule), but they
    // still advance the offset.
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sse_kind.as_deref(), Some("done"));
    assert_eq!(consumed, bytes.len());
}

#[tokio::test]
async fn drain_events_persists_batches_and_tracks_the_offset() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "sess".into(),
            title: None,
            agent: Some("act".into()),
            model: None,
            created_at: 1,
            updated_at: 1,
            workdir_hash: None,
            autopilot_mode: None,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            kind: None,
        })
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.ndjson");
    // A missing file (runner not started yet) is not an error.
    let mut error = None;
    assert_eq!(
        drain_events(&path, 0, &store, "sess", &mut error)
            .await
            .unwrap(),
        0
    );
    std::fs::write(
        &path,
        b"{\"kind\":\"text_delta\",\"payload\":{\"text\":\"a\"}}\n{\"kind\":\"text_delta\",\"payl",
    )
    .unwrap();
    let offset = drain_events(&path, 0, &store, "sess", &mut error)
        .await
        .unwrap();
    assert_eq!(offset as usize, 45);
    assert_eq!(store.events_after("sess", 0).await.unwrap().len(), 1);
    // Complete the second line (as the runner flushing mid-line would).
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    std::io::Write::write_all(&mut file, b"oad\":{\"text\":\"b\"}}\n").unwrap();
    let offset = drain_events(&path, offset, &store, "sess", &mut error)
        .await
        .unwrap();
    let events = store.events_after("sess", 0).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].payload, json!({"text": "b"}));
    assert_eq!(events[1].seq, Some(2));
    assert_eq!(offset as usize, std::fs::read(&path).unwrap().len());
    // No new bytes: offset unchanged, no new rows.
    assert_eq!(
        drain_events(&path, offset, &store, "sess", &mut error)
            .await
            .unwrap(),
        offset
    );
    assert_eq!(store.events_after("sess", 0).await.unwrap().len(), 2);
}
