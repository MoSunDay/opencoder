use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, SessionMeta, Store};
use serde_json::json;

fn event(id: &str, name: &str, kind: &str, at: i64) -> SessionEventRecord {
    SessionEventRecord {
        session_id: id.into(),
        kind: EventKind::Step,
        seq: None,
        ts: at,
        sse_kind: Some(kind.into()),
        payload: json!({"step":name,"at_ms":at,"payload":{"ok":true}}),
    }
}

#[tokio::test]
async fn snapshot_has_latest_attempts_and_exact_cursor_without_log_payloads() {
    let store = LibsqlStore::open_memory().await.unwrap();
    for id in ["run", "other"] {
        store
            .create_session(&SessionMeta {
                id: id.into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    assert_eq!(store.dag_step_snapshot("run").await.unwrap().head_seq, 0);
    let mut rows = vec![
        event("run", "first", "step_started", 1),
        event("run", "first", "step_done", 2),
    ];
    rows.push(event("run", "second", "step_started", 3));
    let mut large = event("run", "first", "step_log", 4);
    large.payload = json!({"step":"first","payload":{"data":{"text":"x".repeat(2 * 1024 * 1024)}}});
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
    assert_eq!(
        snapshot
            .steps
            .iter()
            .find(|step| step.name == "first")
            .unwrap()
            .at_ms,
        5
    );
    let terminal = store
        .append_event(&event("run", "first", "step_done", 7))
        .await
        .unwrap();
    let next = store.dag_step_snapshot("run").await.unwrap();
    assert_eq!(next.head_seq, terminal);
    assert_eq!(
        next.steps
            .iter()
            .find(|step| step.name == "first")
            .unwrap()
            .started_at_ms,
        5
    );
    assert!(
        !next
            .steps
            .iter()
            .find(|step| step.name == "first")
            .unwrap()
            .started
    );
}
