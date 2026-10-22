use super::*;

fn record(id: &str, kind: ExecutionKind, input: Value) -> Record {
    serde_json::from_value(json!({"assignment":{
        "index":{"id":id,"kind":kind,"created_at":1,"node_id":"node-report","status":"done"},
        "request":{"id":id,"kind":kind,"input":input}},
        "annotations":{},"result":null,"error":null,"events":[]
    }))
    .unwrap()
}
fn parent() -> Value {
    json!({"run_id":"brain-parent","operation_id":"op-a","node_id":"step-a","layer":1})
}

#[test]
fn reporting_candidates_follow_durable_save_forget_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(dir.path().into(), None).unwrap();
    let mut journal = Journal::load(layout.clone()).unwrap();
    journal
        .save(record("agent-history", ExecutionKind::Agent, json!({})))
        .unwrap();
    let mut leaf = record(
        "agent-leaf",
        ExecutionKind::Agent,
        json!({"schema_version":7,"brain_layered":{"parent":parent()}}),
    );
    journal.save(leaf.clone()).unwrap();
    assert_eq!(journal.report_candidates.len(), 1);
    let frames = journal.terminal_frames(None, 64).unwrap();
    assert_eq!(frames.len(), 1);
    let NodeFrame::Brain { action, input, .. } = &frames[0] else {
        panic!()
    };
    assert_eq!(action, "layered_terminal");
    assert_eq!(input["source_sequence"], 1);
    leaf.annotations = json!({"brain_layered_ack":1});
    journal.save(leaf.clone()).unwrap();
    let mut loaded = Journal::load(layout).unwrap();
    assert_eq!(loaded.report_candidates.len(), 1);
    assert!(
        loaded.terminal_frames(None, 64).unwrap().is_empty(),
        "durable ACK survives reload"
    );
    leaf.assignment.request.input = json!({});
    loaded.save(leaf).unwrap();
    assert!(loaded.report_candidates.is_empty());
    assert!(journal.forget("agent-leaf").unwrap());
    assert!(journal.report_candidates.is_empty());
}

#[test]
fn terminal_projections_drop_intents_and_keep_only_required_markers() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal =
        Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
    let mut nested = record(
        "brain-nested",
        ExecutionKind::Brain,
        json!({"schema_version":7,"layered_request":{"parent":parent()}}),
    );
    nested.result = json!({"phase":"completed","large":"unused"});
    nested.annotations = json!({"layered_wake_ack":2,"layered_intent":{"assignments":[]},
        "layered_cancel_acks":{"op":true},"layered_dispatch_acks":{"op":true},"layered_guidance_ack":4,
        "unrelated_large_payload":"unused"});
    journal.save(nested).unwrap();
    let frames = journal.terminal_frames(None, 64).unwrap();
    assert_eq!(frames.len(), 1);
    let projected = &journal.records["brain-nested"];
    assert!(projected.annotations.get("layered_intent").is_none());
    assert!(projected
        .annotations
        .get("unrelated_large_payload")
        .is_none());
    assert_eq!(journal.legacy_report_acks().len(), 4);
    journal
        .save(record(
            "agent-invalid",
            ExecutionKind::Agent,
            json!({"schema_version":7,"brain_layered":{}}),
        ))
        .unwrap();
    assert!(
        journal.terminal_frames(None, 64).is_err(),
        "malformed parent bindings must not be silently omitted"
    );
}

#[test]
fn unrelated_history_does_not_enter_periodic_report_views() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal =
        Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
    let benchmark = std::env::var_os("OPENCODER_CPU_BENCH").is_some();
    let payload = if benchmark { 1024 * 1024 } else { 1024 };
    for n in 0..1024 {
        let mut history = record(
            &format!("agent-history-{n}"),
            ExecutionKind::Agent,
            json!({}),
        );
        history.assignment.request.input = json!({"payload":"x".repeat(payload)});
        let path = journal
            .layout
            .record_path(ExecutionKind::Agent, &history.assignment.index.id)
            .unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(&history).unwrap()).unwrap();
        journal
            .insert_loaded(history, RecordLocation::Current, None)
            .unwrap();
    }
    assert!(journal.report_candidates.is_empty());
    let mut samples = Vec::new();
    for _ in 0..100 {
        let start = std::time::Instant::now();
        let frames = journal.terminal_frames(None, 64).unwrap();
        samples.push(start.elapsed());
        assert!(frames.is_empty());
        assert!(!journal.has_terminal_reports());
    }
    samples.sort();
    eprintln!(
        "history_bytes={} empty_brain_p95_us={}",
        1024 * payload,
        samples[94].as_micros()
    );
    assert!(samples[94] < std::time::Duration::from_millis(100));
}

#[tokio::test]
async fn pending_dispatch_reuses_its_generation_across_acks_and_invalidates_new_intents() {
    use opencoder_core::brain::layered::LayeredOperation;
    use opencoder_store::reporting::ReportAck;
    let dir = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(dir.path().into(), None).unwrap();
    let mut journal = Journal::load(layout.clone()).unwrap();
    let operation: LayeredOperation = serde_json::from_value(json!({
        "operation_id":"op","run_id":"brain-root","round":1,"activation":0,"layer":1,
        "node_id":"step","attempt":1,"capability_id":"cap","execution_kind":"agent",
        "execution_id":"agent-child","status":"creating","source_sequence":null,"cancel_requested":false
    })).unwrap();
    let mut root = record(
        "brain-root",
        ExecutionKind::Brain,
        json!({"schema_version":7,"layered_request":{}}),
    );
    root.assignment.index.status = ExecutionStatus::Idle;
    root.annotations = json!({"layered_intent":{"generation":1,"layer":1,
        "operations":[operation],"assignments":[{"node_id":"step","capability_id":"cap","reason":"first"}],
        "capabilities":[{"capability_id":"cap","kind":"agent","target":"act","input_desc":"input","output_desc":"output","definition":{},"version":"1"}]}});
    journal.save(root.clone()).unwrap();
    let path = layout
        .record_path(ExecutionKind::Brain, "brain-root")
        .unwrap();
    let mut cache = None;
    let first = journal
        .dispatch_payload("brain-root", &operation, &mut cache)
        .await
        .unwrap();
    journal
        .apply_report_ack(
            &ReportAck {
                execution_id: "brain-root".into(),
                action: "layered_wake".into(),
                item: String::new(),
                cursor: 1,
            },
            false,
        )
        .unwrap();
    std::fs::write(&path, b"cached generation must not reparse the root").unwrap();
    assert_eq!(
        journal
            .dispatch_payload("brain-root", &operation, &mut cache)
            .await
            .unwrap(),
        first
    );
    root.annotations["layered_intent"]["generation"] = json!(2);
    root.annotations["layered_intent"]["assignments"][0]["reason"] = json!("second");
    journal.save(root).unwrap();
    let changed = journal
        .dispatch_payload("brain-root", &operation, &mut cache)
        .await
        .unwrap();
    assert_eq!(changed["assignment"]["reason"], "second");
}
