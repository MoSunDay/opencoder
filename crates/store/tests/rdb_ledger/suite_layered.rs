//! Brain v4 layered scheduler store: atomic projection commits with
//! generation fencing, operation immutability, the narrow Error→Done
//! recovery lane and wake-receipt dedup (ported from
//! `tests/brain_layered_v4.rs`; the SQL schema-watermark case has no KV
//! counterpart).

use super::layered_fixtures::{created, event, operation, recovery_change, run, stored_error};
use super::*;
use opencoder_core::brain::layered::*;

#[tokio::test]
async fn layered_projection_is_atomic_and_generation_fenced() {
    let store = mem().await;
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
    let path = directory.path().join("layered.rdb");
    let store = RdbStore::open(&path).await.unwrap();
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

    for field in ["round", "activation"] {
        let mut forged = terminal.clone();
        forged.expected_generation = Some(1);
        forged.run.generation = 2;
        if field == "round" {
            forged.operations[0].round += 1;
        } else {
            forged.operations[0].activation += 1;
        }
        assert!(
            store.commit_brain_layered(&forged).await.is_err(),
            "{field} is immutable"
        );
    }

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
    let reopened = RdbStore::open(&path).await.unwrap();
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
    let store = mem().await;
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
    let store = mem().await;
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

#[tokio::test]
async fn current_error_recovery_commits_only_while_stopped() {
    for phase in [LayeredPhase::Paused, LayeredPhase::Blocked] {
        let (store, old) = stored_error(phase, 4, false).await;
        let committed = store
            .commit_brain_layered(&recovery_change(&old))
            .await
            .unwrap();
        assert_eq!(committed.run.phase, phase);
        assert_eq!(committed.run.activation, 4);
        assert_eq!(committed.run.generation, old.run.generation + 1);
        assert_eq!(committed.run.last_event_seq, old.run.last_event_seq + 1);
        assert_eq!(committed.operations[0].status, LayeredOperationStatus::Done);
        assert_eq!(committed.operations[0].source_sequence, Some(6));
        assert_eq!(
            store.brain_layered(&old.run.run_id).await.unwrap(),
            Some(committed)
        );
    }
    for phase in [
        LayeredPhase::Ready,
        LayeredPhase::Waiting,
        LayeredPhase::Deciding,
        LayeredPhase::Completed,
    ] {
        let (store, old) = stored_error(phase, 4, false).await;
        assert!(store
            .commit_brain_layered(&recovery_change(&old))
            .await
            .is_err());
        assert_eq!(
            store.brain_layered(&old.run.run_id).await.unwrap(),
            Some(old)
        );
    }
    // Cancelled attempts and mismatched activations never fold.
    for _cancelled in [true, false] {
        let (store, old) = stored_error(LayeredPhase::Paused, 4, true).await;
        assert!(store
            .commit_brain_layered(&recovery_change(&old))
            .await
            .is_err());
        let (store, old) = stored_error(LayeredPhase::Paused, 3, false).await;
        assert!(store
            .commit_brain_layered(&recovery_change(&old))
            .await
            .is_err());
        drop(store);
    }
    let (store, old) = stored_error(LayeredPhase::Paused, 4, false).await;
    for variant in 0..14 {
        let mut change = recovery_change(&old);
        match variant {
            0 => change.operations[0].source_sequence = Some(4),
            1 => change.operations[0].source_sequence = Some(3),
            2 => change.operations[0].source_sequence = None,
            3 => change.operations[0].status = LayeredOperationStatus::Cancelled,
            4 => change.operations[0].cancel_requested = true,
            5 => change.operations[0].execution_id.push_str("-forged"),
            6 => change.operations[0].activation += 1,
            7 => change.run.phase = LayeredPhase::Ready,
            8 => change.run.activation += 1,
            9 => change.expected_generation = Some(old.run.generation + 1),
            10 => change.operations[0].operation_id.push_str("-forged"),
            11 => change.run.layer += 1,
            12 => change.run.round += 1,
            _ => change.run.valid_layers += 1,
        }
        assert!(
            store.commit_brain_layered(&change).await.is_err(),
            "variant {variant}"
        );
        assert_eq!(
            store.brain_layered(&old.run.run_id).await.unwrap(),
            Some(old.clone())
        );
    }
    let success = store
        .commit_brain_layered(&recovery_change(&old))
        .await
        .unwrap();
    let mut overwrite = recovery_change(&success);
    overwrite.operations[0].status = LayeredOperationStatus::Error;
    assert!(store.commit_brain_layered(&overwrite).await.is_err());
    assert_eq!(
        store.brain_layered(&old.run.run_id).await.unwrap(),
        Some(success)
    );
}

#[tokio::test]
async fn wake_receipt_events_dedup_on_identity_and_allow_null_sequences() {
    let store = mem().await;
    let base = run(LayeredPhase::Ready, 0, 0);
    store
        .commit_brain_layered(&LayeredChange {
            expected_generation: None,
            events: vec![event("run_created", &base)],
            run: base.clone(),
            operations: vec![],
        })
        .await
        .unwrap();
    // First receipt with (execution, source_sequence) identity commits.
    let mut first = LayeredChange {
        expected_generation: Some(0),
        events: vec![],
        run: run(LayeredPhase::Waiting, 1, 1),
        operations: vec![],
    };
    let mut receipt = event("operation_terminal", &first.run);
    receipt.execution_id = Some("ex-a1".into());
    receipt.source_sequence = Some(7);
    first.events.push(receipt.clone());
    store.commit_brain_layered(&first).await.unwrap();
    // The same receipt replayed in a later generation violates the unique
    // index and the whole change rolls back.
    let mut replay = LayeredChange {
        expected_generation: Some(1),
        events: vec![receipt],
        run: run(LayeredPhase::Waiting, 1, 2),
        operations: vec![],
    };
    replay.events[0].run_id = replay.run.run_id.clone();
    assert!(store.commit_brain_layered(&replay).await.is_err());
    let snapshot = store
        .brain_layered("brain-layered-store")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.run.generation, 1);
    assert_eq!(
        store
            .brain_layered_events("brain-layered-store", 0, 100)
            .await
            .unwrap()
            .len(),
        2
    );
    // A duplicate inside ONE change is rejected the same way.
    let mut dupe = LayeredChange {
        expected_generation: Some(1),
        events: vec![],
        run: run(LayeredPhase::Waiting, 1, 2),
        operations: vec![],
    };
    let mut twice = event("operation_terminal", &dupe.run);
    twice.execution_id = Some("ex-a2".into());
    twice.source_sequence = Some(3);
    dupe.events = vec![twice.clone(), twice];
    assert!(store.commit_brain_layered(&dupe).await.is_err());
    // NULL source sequences are distinct inside SQLite unique indexes, so
    // receipts without one may repeat (within one change and across them).
    let mut nulls = LayeredChange {
        expected_generation: Some(1),
        events: vec![],
        run: run(LayeredPhase::Waiting, 1, 2),
        operations: vec![],
    };
    let mut anonymous = event("note", &nulls.run);
    anonymous.execution_id = Some("ex-a1".into());
    nulls.events = vec![anonymous.clone(), anonymous];
    store.commit_brain_layered(&nulls).await.unwrap();
}
