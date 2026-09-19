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
