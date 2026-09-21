use opencoder_core::brain::layered::*;
use opencoder_core::fleet::ExecutionKind;
use opencoder_store::Store;

fn run(phase: LayeredPhase, layer: u32, generation: u64) -> LayeredRun {
    LayeredRun {
        run_id: "brain-layered-store".into(),
        phase,
        layer,
        generation,
        last_event_seq: 0,
        error: None,
        summary: None,
        depth: 0,
        parent: None,
        created_at: 1,
        updated_at: 1,
    }
}

fn event(event_type: &str, run: &LayeredRun) -> LayeredEvent {
    LayeredEvent {
        seq: 0,
        run_id: run.run_id.clone(),
        layer: run.layer,
        event_type: event_type.into(),
        node_id: None,
        attempt: None,
        capability_id: None,
        execution_kind: None,
        execution_id: None,
        decision_summary: None,
        reason_summary: None,
        source_sequence: None,
        evidence_execution_ids: vec![],
        at_ms: 1,
    }
}

fn created() -> LayeredChange {
    let run = run(LayeredPhase::Ready, 0, 0);
    LayeredChange {
        expected_generation: None,
        events: vec![event("run_created", &run)],
        run,
        operations: vec![],
    }
}

fn operation(
    attempt: u32,
    status: LayeredOperationStatus,
    sequence: Option<u64>,
) -> LayeredOperation {
    LayeredOperation {
        operation_id: format!("brain-layered-store#l1#impact#a{attempt}"),
        run_id: "brain-layered-store".into(),
        layer: 1,
        node_id: "impact".into(),
        attempt,
        capability_id: "agent-impact".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: format!("agent-a{attempt}"),
        status,
        source_sequence: sequence,
        cancel_requested: false,
    }
}

#[tokio::test]
async fn layered_projection_is_atomic_and_generation_fenced() {
    let store = opencoder_store::LibsqlStore::open_memory().await.unwrap();
    let first = store.commit_brain_layered(&created()).await.unwrap();
    assert_eq!(first.schema_version, LAYERED_SCHEMA_VERSION);
    assert_eq!(first.run.last_event_seq, 1);

    let mut stale = created();
    stale.expected_generation = Some(0);
    stale.run.generation = 1;
    stale.run.phase = LayeredPhase::Deciding;
    let second = store.commit_brain_layered(&stale).await.unwrap();
    assert_eq!(second.run.phase, LayeredPhase::Deciding);
    assert!(store.commit_brain_layered(&stale).await.is_err());

    let events = store
        .brain_layered_events("brain-layered-store", 0, 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "run_created");
    assert_eq!(events[1].seq, 2);
    assert_eq!(
        store
            .brain_layered_events("brain-layered-store", 1, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn terminal_attempt_is_immutable_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("layered.db");
    let store = opencoder_store::LibsqlStore::open(&path).await.unwrap();
    store.commit_brain_layered(&created()).await.unwrap();

    let mut terminal = created();
    terminal.expected_generation = Some(0);
    terminal.run.generation = 1;
    terminal.run.phase = LayeredPhase::Waiting;
    terminal.run.layer = 1;
    terminal
        .operations
        .push(operation(1, LayeredOperationStatus::Done, Some(9)));
    let mut folded = event("operation_terminal", &terminal.run);
    folded.execution_id = Some("agent-a1".into());
    folded.source_sequence = Some(9);
    terminal.events.push(folded);
    store.commit_brain_layered(&terminal).await.unwrap();

    // A folded terminal attempt cannot be rewritten.
    let mut rewrite = terminal.clone();
    rewrite.expected_generation = Some(1);
    rewrite.run.generation = 2;
    rewrite.operations[0].status = LayeredOperationStatus::Error;
    assert!(store.commit_brain_layered(&rewrite).await.is_err());

    // ... and the run cannot leave a terminal phase.
    let mut completed = terminal.clone();
    completed.expected_generation = Some(1);
    completed.run.generation = 2;
    completed.run.phase = LayeredPhase::Completed;
    completed.run.summary = Some("done".into());
    completed.events = vec![event("run_completed", &completed.run)];
    let completed = store.commit_brain_layered(&completed).await.unwrap();
    let mut reopen = terminal.clone();
    reopen.expected_generation = Some(2);
    reopen.run.generation = 3;
    reopen.run.phase = LayeredPhase::Waiting;
    reopen.events = vec![];
    assert!(store.commit_brain_layered(&reopen).await.is_err());
    assert_eq!(completed.run.phase, LayeredPhase::Completed);

    drop(store);
    let reopened = opencoder_store::LibsqlStore::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .brain_layered("brain-layered-store")
            .await
            .unwrap()
            .unwrap(),
        completed
    );
}

#[tokio::test]
async fn retry_attempts_are_stored_as_separate_operations() {
    let store = opencoder_store::LibsqlStore::open_memory().await.unwrap();
    let mut retried = created();
    retried.run.phase = LayeredPhase::Waiting;
    retried.run.layer = 1;
    retried
        .operations
        .push(operation(1, LayeredOperationStatus::Error, Some(1)));
    retried
        .operations
        .push(operation(2, LayeredOperationStatus::Creating, None));
    let snapshot = store.commit_brain_layered(&retried).await.unwrap();
    assert_eq!(snapshot.operations.len(), 2);
    let live = snapshot
        .operations
        .iter()
        .max_by_key(|op| op.attempt)
        .unwrap();
    assert_eq!(live.attempt, 2);
    assert_eq!(live.status, LayeredOperationStatus::Creating);
    assert_eq!(
        store
            .brain_layered("brain-layered-store")
            .await
            .unwrap()
            .unwrap(),
        snapshot
    );
}

#[tokio::test]
async fn unknown_run_reads_as_none_and_forged_operations_rejected() {
    let store = opencoder_store::LibsqlStore::open_memory().await.unwrap();
    assert!(store
        .brain_layered("brain-missing")
        .await
        .unwrap()
        .is_none());

    let mut foreign = created();
    let mut op = operation(1, LayeredOperationStatus::Creating, None);
    op.run_id = "brain-other".into();
    foreign.operations.push(op);
    assert!(store.commit_brain_layered(&foreign).await.is_err());

    let mut ahead = created();
    ahead.run.layer = 0;
    let mut op = operation(1, LayeredOperationStatus::Creating, None);
    op.layer = 3;
    ahead.operations.push(op);
    assert!(store.commit_brain_layered(&ahead).await.is_err());
}

#[test]
fn store_schema_version_is_untouched() {
    // v4 tables bootstrap unconditionally, so they must not move the
    // database watermark: 28 is the release line's own bump (operator config
    // plane), and a v4-induced bump would fail here.
    assert_eq!(opencoder_store::libsql_store::schema_watermark(), 28);
}
