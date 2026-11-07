//! Frozen DAG step snapshots: latest lifecycle row per step, the exact
//! session event cursor, started watermarks, ok/error fidelity and payload
//! isolation. Ported from `tests/dag_snapshot.rs`; the contract is read-only
//! (a snapshot is derived from the uploaded event log, never written back),
//! and rdb seqs are per-session rather than global.
//!
//! Row shape: `sse_kind` selects the lifecycle row (`step_started` /
//! `step_done`), `payload.step` names the step, `payload.at_ms` overrides
//! `ts`, and `payload.payload.{ok,error}` carry the outcome.

use super::*;
use opencoder_store::{EventKind, SessionEventRecord, SessionMeta};
use serde_json::json;

fn session(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        ..Default::default()
    }
}

fn event(id: &str, name: &str, kind: &str, at: i64) -> SessionEventRecord {
    SessionEventRecord {
        session_id: id.into(),
        kind: EventKind::Step,
        seq: None,
        ts: at,
        sse_kind: Some(kind.into()),
        payload: json!({"step": name, "at_ms": at, "payload": {"ok": true}}),
    }
}

/// Lifecycle row without `at_ms` (the reader falls back to `ts`) and with an
/// arbitrary outcome body: exact JSON fidelity is the assertion.
fn event_body(
    id: &str,
    name: &str,
    kind: &str,
    ts: i64,
    body: serde_json::Value,
) -> SessionEventRecord {
    SessionEventRecord {
        session_id: id.into(),
        kind: EventKind::Step,
        seq: None,
        ts,
        sse_kind: Some(kind.into()),
        payload: json!({"step": name, "payload": body}),
    }
}

fn find<'a>(
    snapshot: &'a opencoder_store::store::dag_snapshot::DagStepSnapshot,
    name: &str,
) -> &'a opencoder_store::store::dag_snapshot::DagStepEvent {
    snapshot
        .steps
        .iter()
        .find(|step| step.name == name)
        .unwrap_or_else(|| panic!("step {name} missing from snapshot"))
}

#[tokio::test]
async fn snapshot_has_latest_attempts_and_exact_cursor_without_log_payloads() {
    let store = mem().await;
    for id in ["run", "other"] {
        store.create_session(&session(id)).await.unwrap();
    }
    let empty = store.dag_step_snapshot("run").await.unwrap();
    assert_eq!(empty.head_seq, 0);
    assert!(empty.steps.is_empty());

    let mut rows = vec![
        event("run", "first", "step_started", 1),
        event("run", "first", "step_done", 2),
        event("run", "second", "step_started", 3),
    ];
    // A 2 MiB step log is never read: only lifecycle rows become steps.
    let mut large = event("run", "first", "step_log", 4);
    large.payload =
        json!({"step": "first", "payload": {"data": {"text": "x".repeat(2 * 1024 * 1024)}}});
    rows.push(large);
    rows.push(event("run", "first", "step_started", 5));
    let seqs = store.append_events(&rows).await.unwrap();
    store
        .append_event(&event("other", "first", "step_done", 6))
        .await
        .unwrap();

    let snapshot = store.dag_step_snapshot("run").await.unwrap();
    assert_eq!(snapshot.head_seq, *seqs.last().unwrap());
    assert_eq!(snapshot.steps.len(), 2);
    assert!(snapshot.steps.iter().all(|step| step.started));
    let first = find(&snapshot, "first");
    assert_eq!(first.seq, 5, "a re-run replaces the earlier attempt row");
    assert_eq!(first.at_ms, 5);
    assert_eq!(
        first.started_at_ms, 5,
        "started watermark is the latest start"
    );
    // The other session's append did not move this session's cursor.
    assert_eq!(store.dag_step_snapshot("other").await.unwrap().head_seq, 1);

    let terminal = store
        .append_event(&event("run", "first", "step_done", 7))
        .await
        .unwrap();
    let next = store.dag_step_snapshot("run").await.unwrap();
    assert_eq!(next.head_seq, terminal);
    let first = find(&next, "first");
    assert!(!first.started, "the latest row is terminal");
    assert_eq!(first.at_ms, 7);
    assert_eq!(
        first.started_at_ms, 5,
        "the started watermark survives termination"
    );
    assert_eq!(find(&next, "second").started_at_ms, 3);
}

#[tokio::test]
async fn snapshot_keeps_outcome_fidelity_and_ignores_non_lifecycle_rows() {
    let store = mem().await;
    store.create_session(&session("run")).await.unwrap();
    let mut skipped = event("run", "skipped", "step_started", 5);
    skipped.sse_kind = None;
    let mut unnamed = event("run", "ignored", "step_started", 6);
    unnamed.payload = json!({"at_ms": 6});
    let rows = vec![
        event("run", "build", "step_started", 10),
        event_body(
            "run",
            "build",
            "step_done",
            20,
            json!({"ok": false, "error": "boom\n— ünïcode"}),
        ),
        event_body("run", "probe", "step_started", 30, json!({})),
        event_body("run", "probe", "step_done", 40, json!({"ok": 0})),
        event_body(
            "run",
            "pick",
            "step_done",
            50,
            json!({"ok": 1, "error": null}),
        ),
        skipped,
        unnamed,
    ];
    let seqs = store.append_events(&rows).await.unwrap();
    let snapshot = store.dag_step_snapshot("run").await.unwrap();
    assert_eq!(snapshot.head_seq, *seqs.last().unwrap());
    assert_eq!(
        snapshot
            .steps
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        vec!["build", "probe", "pick"],
        "steps are ordered by their latest row seq"
    );

    let build = find(&snapshot, "build");
    assert_eq!(build.seq, 2);
    assert!(!build.started);
    assert_eq!(build.at_ms, 20);
    assert_eq!(build.started_at_ms, 10);
    assert!(!build.ok);
    assert_eq!(build.error.as_deref(), Some("boom\n— ünïcode"));

    // `at_ms` absent: the reader falls back to the event timestamp.
    let probe = find(&snapshot, "probe");
    assert_eq!(probe.at_ms, 40);
    assert_eq!(probe.started_at_ms, 30);
    assert!(!probe.ok, "json truthiness: integer 0 is false");
    assert_eq!(probe.error, None);

    // Integer 1 is true and a null error is no error.
    let pick = find(&snapshot, "pick");
    assert!(!pick.started);
    assert_eq!(pick.at_ms, 50);
    assert_eq!(pick.started_at_ms, 0);
    assert!(pick.ok);
    assert_eq!(pick.error, None);

    // A step name that never appeared is simply absent, and an unknown
    // session is an empty snapshot, not an error.
    assert!(snapshot.steps.iter().all(|step| step.name != "missing"));
    let absent = store.dag_step_snapshot("absent").await.unwrap();
    assert_eq!(absent.head_seq, 0);
    assert!(absent.steps.is_empty());
}

#[tokio::test]
async fn snapshot_is_a_pure_read_of_the_event_log() {
    let store = mem().await;
    store.create_session(&session("run")).await.unwrap();
    let seq = store
        .append_event(&event("run", "first", "step_started", 1))
        .await
        .unwrap();

    let first = store.dag_step_snapshot("run").await.unwrap();
    let second = store.dag_step_snapshot("run").await.unwrap();
    assert_eq!(first.head_seq, second.head_seq);
    assert_eq!(first.steps.len(), second.steps.len());
    assert_eq!(first.steps[0].seq, second.steps[0].seq);

    // Reading does not consume or rewrite the log: the replay cursor and the
    // stored rows are unchanged.
    assert_eq!(store.last_event_seq("run").await.unwrap(), seq);
    assert_eq!(store.events_after("run", 0).await.unwrap().len(), 1);
    let later = store
        .append_event(&event("run", "first", "step_done", 2))
        .await
        .unwrap();
    assert_eq!(
        store.dag_step_snapshot("run").await.unwrap().head_seq,
        later
    );
    assert_eq!(store.dag_step_snapshot("absent").await.unwrap().head_seq, 0);
    assert_eq!(store.last_event_seq("absent").await.unwrap(), 0);
}
