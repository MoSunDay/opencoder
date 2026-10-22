use super::*;
use opencoder_store::{reporting::ReportAck, TursoStore};

fn ack(action: &str, item: &str, cursor: u64) -> ReportAck {
    ReportAck {
        execution_id: "brain-layered-store".into(),
        action: action.into(),
        item: item.into(),
        cursor,
    }
}

#[tokio::test]
async fn pending_reports_follow_commits_and_acknowledgements_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reports.db");
    let store = TursoStore::open(&path).await.unwrap();
    store.commit_brain_layered(&created()).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "layered_wake");
    store.report_ack(&ack("layered_wake", "", 0)).await.unwrap();
    assert!(!store.has_pending_reports().await.unwrap());

    let mut next = created();
    next.expected_generation = Some(0);
    next.run.generation = 1;
    next.run.phase = LayeredPhase::Waiting;
    next.run.layer = 1;
    next.operations
        .push(operation(1, LayeredOperationStatus::Creating, None));
    store.commit_brain_layered(&next).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "layered_dispatch");
    store
        .report_ack(&ack(
            "layered_dispatch",
            &next.operations[0].operation_id,
            1,
        ))
        .await
        .unwrap();

    next.expected_generation = Some(1);
    next.run.generation = 2;
    next.operations[0].cancel_requested = true;
    store.commit_brain_layered(&next).await.unwrap();
    assert_eq!(
        store.pending_reports(None, 64).await.unwrap()[0].action,
        "layered_cancel"
    );
    assert!(store.commit_brain_layered(&next).await.is_err());
    assert_eq!(store.pending_reports(None, 64).await.unwrap().len(), 1);
    drop(store);

    let store = TursoStore::open(&path).await.unwrap();
    store.reporting_recover().await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "layered_cancel");
    assert_eq!(store.report_acks().await.unwrap().len(), 2);
    next.expected_generation = Some(2);
    next.run.generation = 3;
    next.run.phase = LayeredPhase::Cancelled;
    store.commit_brain_layered(&next).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].action, "settle",
        "terminal roots no longer send operation frames"
    );
}

#[tokio::test]
async fn guidance_is_sequential_and_duplicate_acks_do_not_write() {
    let store = TursoStore::open_memory().await.unwrap();
    let mut initial = created();
    initial.run.phase = LayeredPhase::Waiting;
    initial.events.clear();
    for message in ["first", "second"] {
        let mut e = event("guidance_processed", &initial.run);
        e.guidance.push(LayeredGuidance {
            execution_id: "agent-child".into(),
            message: message.into(),
        });
        initial.events.push(e);
    }
    store.commit_brain_layered(&initial).await.unwrap();
    let pending = store.pending_reports(None, 64).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].cursor, 1);
    assert_eq!(pending[0].payload["guidance"][0]["message"], "first");
    store
        .report_ack(&ack("layered_guidance", "", 1))
        .await
        .unwrap();
    let next = store
        .pending_reports(Some(&pending[0].key), 64)
        .await
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].cursor, 2);
    store
        .report_ack(&ack("layered_guidance", "", 2))
        .await
        .unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute("CREATE TRIGGER no_ack_insert BEFORE INSERT ON execution_report_acks BEGIN SELECT RAISE(ABORT,'duplicate ACK write'); END", ()).await.unwrap();
    conn.execute("CREATE TRIGGER no_ack_update BEFORE UPDATE ON execution_report_acks BEGIN SELECT RAISE(ABORT,'duplicate ACK write'); END", ()).await.unwrap();
    store
        .report_ack(&ack("layered_guidance", "", 2))
        .await
        .unwrap();
    store
        .report_ack(&ack("layered_guidance", "", 1))
        .await
        .unwrap();
    assert!(!store.has_pending_reports().await.unwrap());
    assert_eq!(store.report_acks().await.unwrap()[0].cursor, 2);
}

#[tokio::test]
async fn report_projection_failure_rolls_back_the_brain_commit() {
    let store = TursoStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute("CREATE TRIGGER fail_report BEFORE INSERT ON execution_report_queue BEGIN SELECT RAISE(ABORT,'injected failure'); END", ()).await.unwrap();
    assert!(store.commit_brain_layered(&created()).await.is_err());
    assert!(store
        .brain_layered("brain-layered-store")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .report_run_state("brain-layered-store")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .brain_layered_events("brain-layered-store", 0, 10)
        .await
        .unwrap()
        .is_empty());
    conn.execute("DROP TRIGGER fail_report", ()).await.unwrap();
    store.commit_brain_layered(&created()).await.unwrap();
    assert!(store.has_pending_reports().await.unwrap());
}

#[tokio::test]
async fn legacy_projection_recovery_is_repeatable_and_format_rejects_old_readers() {
    let store = TursoStore::open_memory().await.unwrap();
    store.commit_brain_layered(&created()).await.unwrap();
    store.report_ack(&ack("layered_wake", "", 0)).await.unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute("DELETE FROM execution_report_runs", ())
        .await
        .unwrap();
    store.reporting_recover().await.unwrap();
    store.reporting_recover().await.unwrap();
    assert!(!store.has_pending_reports().await.unwrap());
    store.reporting_format(1, 0).await.unwrap();
    store.reporting_format(0, 0).await.unwrap();
    store.reporting_format(1, 1).await.unwrap();
    assert!(store.reporting_format(0, 0).await.is_err());
    store.reporting_format(1, 0).await.unwrap();
    assert!(store.reporting_format(0, 0).await.is_err());
}

#[tokio::test]
async fn bounded_selection_ignores_unselected_corrupt_payloads() {
    let store = TursoStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    for i in 0..130 {
        conn.execute(
            "INSERT INTO execution_report_queue(key,execution_id,action,item,cursor,payload) VALUES(?1,?1,'layered_wake','',0,?2)",
            turso::params![
                format!("key-{i:03}"),
                if i == 129 { "invalid JSON" } else { "{}" }
            ],
        )
        .await
        .unwrap();
    }
    let first = store.pending_reports(None, 1000).await.unwrap();
    assert_eq!(first.len(), 64);
    let second = store
        .pending_reports(Some(&first[63].key), 64)
        .await
        .unwrap();
    assert_eq!(second.len(), 64);
    assert_eq!(second[0].key, "key-064");
    assert!(store
        .pending_reports(Some(&second[63].key), 64)
        .await
        .is_err());
    assert!(store.has_pending_reports().await.unwrap());
}

#[tokio::test]
async fn legacy_guidance_queue_migrates_to_indexed_heads_and_ack_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guidance.db");
    let store = TursoStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute("DROP TABLE execution_report_queue", ())
        .await
        .unwrap();
    conn.execute("CREATE TABLE execution_report_queue(key TEXT PRIMARY KEY,execution_id TEXT NOT NULL,action TEXT NOT NULL,item TEXT NOT NULL,cursor INTEGER NOT NULL,payload TEXT NOT NULL,UNIQUE(execution_id,action,item))", ()).await.unwrap();
    for cursor in 1..=3 {
        conn.execute("INSERT INTO execution_report_queue VALUES(?1,'brain-layered-store','layered_guidance',?1,?2,'{}')", turso::params![format!("key-{cursor}"), cursor]).await.unwrap();
    }
    drop(conn);
    drop(store);
    let store = TursoStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    let mut plan = conn.query("EXPLAIN QUERY PLAN SELECT key,payload FROM execution_report_queue WHERE eligible=1 AND key>'' ORDER BY key LIMIT 64", ()).await.unwrap();
    let mut details = String::new();
    while let Some(row) = plan.next().await.unwrap() {
        details.push_str(&row.get::<String>(3).unwrap());
    }
    assert!(details.contains("execution_reports_ready"), "{details}");
    assert_eq!(store.pending_reports(None, 64).await.unwrap()[0].cursor, 1);
    assert_eq!(store.pending_reports(None, 64).await.unwrap().len(), 1);
    conn.execute("CREATE TRIGGER fail_promotion BEFORE UPDATE OF eligible ON execution_report_queue BEGIN SELECT RAISE(ABORT,'injected promotion failure'); END", ()).await.unwrap();
    assert!(store
        .report_ack(&ack("layered_guidance", "", 1))
        .await
        .is_err());
    assert!(store.report_acks().await.unwrap().is_empty());
    assert_eq!(store.pending_reports(None, 64).await.unwrap()[0].cursor, 1);
    conn.execute("DROP TRIGGER fail_promotion", ())
        .await
        .unwrap();
    store
        .report_ack(&ack("layered_guidance", "", 1))
        .await
        .unwrap();
    assert_eq!(store.pending_reports(None, 64).await.unwrap()[0].cursor, 2);
    drop(conn);
    drop(store);
    let reopened = TursoStore::open(&path).await.unwrap();
    assert_eq!(reopened.pending_reports(None, 64).await.unwrap().len(), 1);
    assert_eq!(
        reopened.pending_reports(None, 64).await.unwrap()[0].cursor,
        2
    );
}
