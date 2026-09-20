use opencoder_core::brain::*;
use opencoder_store::Store;
use serde_json::json;

fn change() -> BrainSchedulerChange {
    let run = BrainSchedulerRun {
        run_id: "brain-store-v3".into(),
        phase: BrainSchedulerPhase::Ready,
        round: 0,
        generation: 0,
        last_event_seq: 0,
        error: None,
        created_at: 1,
        updated_at: 1,
    };
    BrainSchedulerChange {
        expected_generation: None,
        run: run.clone(),
        operations: vec![],
        events: vec![BrainSchedulerEvent {
            seq: 0,
            run_id: run.run_id.clone(),
            round: 0,
            event_type: "run_created".into(),
            capability_id: None,
            execution_kind: None,
            execution_id: None,
            decision_summary: None,
            reason_summary: None,
            source_sequence: None,
            evidence_execution_ids: vec![],
            at_ms: 1,
        }],
    }
}
#[tokio::test]
async fn scheduler_projection_is_atomic_and_generation_fenced() {
    let store = opencoder_store::LibsqlStore::open_memory().await.unwrap();
    let first = store.commit_brain_scheduler(&change()).await.unwrap();
    assert_eq!(first.run.last_event_seq, 1);
    let mut stale = change();
    stale.run.generation = 1;
    stale.expected_generation = Some(0);
    stale.run.phase = BrainSchedulerPhase::Paused;
    let second = store.commit_brain_scheduler(&stale).await.unwrap();
    assert_eq!(second.run.generation, 1);
    assert!(store.commit_brain_scheduler(&stale).await.is_err());
    let events = store
        .brain_scheduler_events("brain-store-v3", 0, 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
}
#[tokio::test]
async fn scheduler_event_does_not_contain_execution_body() {
    let store = opencoder_store::LibsqlStore::open_memory().await.unwrap();
    let mut c = change();
    c.events[0].reason_summary = Some("summary only".into());
    store.commit_brain_scheduler(&c).await.unwrap();
    let event = store
        .brain_scheduler_events("brain-store-v3", 0, 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(event.reason_summary, Some("summary only".into()));
    assert!(serde_json::to_string(&event)
        .unwrap()
        .find("execution body")
        .is_none());
    let _ = json!({"ok":true});
}

#[tokio::test]
async fn terminal_dedup_rolls_back_projection_and_survives_reopen() {
    use opencoder_core::fleet::ExecutionKind;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scheduler.db");
    let store = opencoder_store::LibsqlStore::open(&path).await.unwrap();
    store.commit_brain_scheduler(&change()).await.unwrap();
    let mut terminal = change();
    terminal.expected_generation = Some(0);
    terminal.run.generation = 1;
    terminal.run.round = 1;
    terminal.operations.push(BrainOperation {
        operation_id: "operation-a".into(),
        run_id: terminal.run.run_id.clone(),
        round: 1,
        capability_id: "a".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: "agent-a".into(),
        status: BrainOperationStatus::Done,
        source_sequence: Some(9),
        cancel_requested: false,
    });
    let event = &mut terminal.events[0];
    event.event_type = "operation_terminal".into();
    event.execution_id = Some("agent-a".into());
    event.source_sequence = Some(9);
    let saved = store.commit_brain_scheduler(&terminal).await.unwrap();
    let mut replay = terminal.clone();
    replay.expected_generation = Some(1);
    replay.run.generation = 2;
    replay.run.phase = BrainSchedulerPhase::Paused;
    replay.operations[0].cancel_requested = true;
    assert!(store.commit_brain_scheduler(&replay).await.is_err());
    assert_eq!(
        store
            .brain_scheduler(&terminal.run.run_id)
            .await
            .unwrap()
            .unwrap(),
        saved
    );
    drop(store);
    let reopened = opencoder_store::LibsqlStore::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .brain_scheduler(&terminal.run.run_id)
            .await
            .unwrap()
            .unwrap(),
        saved
    );
    let first = reopened
        .brain_scheduler_events(&terminal.run.run_id, 0, 1)
        .await
        .unwrap();
    let second = reopened
        .brain_scheduler_events(&terminal.run.run_id, first[0].seq, 1)
        .await
        .unwrap();
    assert_eq!(first[0].event_type, "run_created");
    assert_eq!(second[0].event_type, "operation_terminal");
    assert!(reopened
        .brain_scheduler_events(&terminal.run.run_id, second[0].seq, 1)
        .await
        .unwrap()
        .is_empty());
}
