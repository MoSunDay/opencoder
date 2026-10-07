use super::*;

#[tokio::test]
async fn recorded_state_machine_pending_promote_mark() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let seq = store
        .admit_input(&mk_input("s", "in-1", Delivery::Steer))
        .await
        .unwrap();

    // Admitted → pending: promoted NULL, recorded=0.
    {
        let conn = store.conn().await.unwrap();
        assert_eq!(input_state(&conn, seq).await, (None, 0));
    }
    assert_eq!(
        store
            .pending_inputs("s", Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1
    );

    // Promoted: promoted_seq set, still recorded=0 (promote ≠ consume).
    let promoted = store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    assert_eq!(promoted, vec![seq]);
    {
        let conn = store.conn().await.unwrap();
        let (p, recorded) = input_state(&conn, seq).await;
        assert!(p.is_some(), "promoted_seq must be set after promote");
        assert_eq!(recorded, 0, "promote must leave recorded=0");
    }
    assert!(store
        .pending_inputs("s", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());

    // Recorded: durably consumed. Idempotent — re-marking is a no-op.
    store.mark_inputs_recorded("s", &[seq]).await.unwrap();
    store.mark_inputs_recorded("s", &[]).await.unwrap();
    store.mark_inputs_recorded("s", &[seq]).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        let (p, recorded) = input_state(&conn, seq).await;
        assert!(p.is_some());
        assert_eq!(recorded, 1, "mark_inputs_recorded must set recorded=1");
    }
    // Still excluded from pending throughout.
    assert!(store
        .pending_inputs("s", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn promote_resets_recorded_marker_on_repromotion() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let seq = store
        .admit_input(&mk_input("s", "in-1", Delivery::Steer))
        .await
        .unwrap();
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    store.mark_inputs_recorded("s", &[seq]).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        assert_eq!(input_state(&conn, seq).await.1, 1);
    }

    // Unpromote only clears promoted_seq; the recorded marker is left as-is
    // (it is promote's job to reset it).
    store.unpromote_inputs("s", &[seq]).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        assert_eq!(
            input_state(&conn, seq).await,
            (None, 1),
            "unpromote clears promoted_seq but must not touch recorded"
        );
    }

    // Re-promote: the stale marker must be reset to 0.
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        let (p, recorded) = input_state(&conn, seq).await;
        assert!(p.is_some());
        assert_eq!(recorded, 0, "re-promotion must reset recorded to 0");
    }
}

#[tokio::test]
async fn recover_orphan_inputs_recovers_only_unrecorded_promoted_rows() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    // Three steers with admitted_seq 1..3 (admit assigns them in order).
    let seq_a = store
        .admit_input(&mk_input("s", "a", Delivery::Steer))
        .await
        .unwrap();
    let seq_b = store
        .admit_input(&mk_input("s", "b", Delivery::Steer))
        .await
        .unwrap();
    let seq_c = store
        .admit_input(&mk_input("s", "c", Delivery::Steer))
        .await
        .unwrap();

    // a: promoted + recorded (fully consumed).
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    store.mark_inputs_recorded("s", &[seq_a]).await.unwrap();
    // b: promoted but NEVER recorded — the orphan (crash between promote and
    // consume). c: never promoted, still pending.
    store.promote_inputs("s", 2, Delivery::Steer).await.unwrap();

    assert_eq!(
        store
            .pending_inputs("s", Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1,
        "only the never-promoted row c is pending pre-recovery"
    );

    // Recovery returns exactly the one orphan.
    let recovered = store.recover_orphan_inputs("s").await.unwrap();
    assert_eq!(recovered, 1, "exactly the promoted+unrecorded row recovers");

    // The orphan is pending again; the recorded row is not.
    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 2, "orphan b rejoins pending alongside c");
    assert!(pending.iter().any(|i| i.seq == Some(seq_b)));
    assert!(pending.iter().any(|i| i.seq == Some(seq_c)));
    {
        let conn = store.conn().await.unwrap();
        let (p_a, rec_a) = input_state(&conn, seq_a).await;
        assert!(p_a.is_some(), "recorded row a must stay promoted");
        assert_eq!(rec_a, 1, "recorded row a must stay recorded");
        assert_eq!(
            input_state(&conn, seq_b).await,
            (None, 0),
            "orphan b must be pending with recorded=0"
        );
    }

    // Idempotent: nothing left to recover.
    assert_eq!(store.recover_orphan_inputs("s").await.unwrap(), 0);
    // Unknown session: no error, zero rows.
    assert_eq!(store.recover_orphan_inputs("nope").await.unwrap(), 0);
}

#[tokio::test]
async fn migration_v9_to_v10_backfills_recorded_for_promoted_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-recorded.db");

    let (seq_promoted, seq_pending);
    {
        let store = LibsqlStore::open(&db_path).await.unwrap();
        make_session(&store, "s", 1).await;
        seq_promoted = store
            .admit_input(&mk_input("s", "legacy-promoted", Delivery::Steer))
            .await
            .unwrap();
        seq_pending = store
            .admit_input(&mk_input("s", "legacy-pending", Delivery::Steer))
            .await
            .unwrap();
        store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();

        // Rewind to a faithful v9 state: version 9, and the promoted row
        // carrying recorded=0 as it would before the column existed (the
        // pending row already has recorded=0).
        let conn = store.conn().await.unwrap();
        // Project tables did not exist at v9; do not carry their v32 shape
        // into this historical fixture.
        conn.execute_batch(
            "DROP TABLE project_todo_executions;
             DROP TABLE project_todo_runs;
             DROP TABLE project_todo_tags;
             DROP TABLE project_tags;
             DROP TABLE project_todos;
             DROP TABLE project_initiatives;
             DROP TABLE project_goals;",
        )
        .await
        .unwrap();
        conn.execute("UPDATE schema_version SET version = 9", ())
            .await
            .unwrap();
        conn.execute(
            "UPDATE session_inputs SET recorded = 0 WHERE promoted_seq IS NOT NULL",
            (),
        )
        .await
        .unwrap();
    }

    // Reopen: bootstrap sees version 9 < 10 and runs the migration.
    let store = LibsqlStore::open(&db_path).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let v: i64 = rows
            .next()
            .await
            .unwrap()
            .expect("version row")
            .get(0)
            .unwrap();
        assert_eq!(v, 33, "schema version must be latest (33) after migration");

        // Backfill: pre-existing promoted row treated as consumed.
        let (p_prom, rec_prom) = input_state(&conn, seq_promoted).await;
        assert!(p_prom.is_some(), "legacy promoted row keeps promoted_seq");
        assert_eq!(rec_prom, 1, "legacy promoted row backfills to recorded=1");
        // Pending row untouched: still pending, recorded=0.
        assert_eq!(
            input_state(&conn, seq_pending).await,
            (None, 0),
            "pending row keeps recorded=0"
        );
    }

    // The backfilled row is not an orphan: recovery finds nothing, and the
    // pending row is still visible for the next drain.
    assert_eq!(
        store.recover_orphan_inputs("s").await.unwrap(),
        0,
        "backfilled promoted rows must not be recoverable orphans"
    );
    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].seq, Some(seq_pending));
}

#[tokio::test]
async fn claim_next_queue_never_reclaims_recorded_rows() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let seq1 = store
        .admit_input(&mk_input("s", "q-1", Delivery::Queue))
        .await
        .unwrap();
    let seq2 = store
        .admit_input(&mk_input("s", "q-2", Delivery::Queue))
        .await
        .unwrap();

    // Consume row 1: claim + record, then simulate the error-recovery
    // unpromote that returns the row to pending without touching recorded.
    let (claimed1, input1) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("first claim serves the oldest pending row");
    assert_eq!(claimed1, seq1);
    assert_eq!(input1.id, "q-1");
    store.mark_inputs_recorded("s", &[seq1]).await.unwrap();
    store.unpromote_inputs("s", &[seq1]).await.unwrap();
    {
        let conn = store.conn().await.unwrap();
        assert_eq!(
            input_state(&conn, seq1).await,
            (None, 1),
            "fixture: row 1 is pending again but stays recorded=1"
        );
    }

    // The second claim must serve row 2 — row 1 is consumed, not pending.
    let (claimed2, input2) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("second claim must serve the genuinely pending row 2");
    assert_eq!(
        claimed2, seq2,
        "claim must skip the recorded=1 row and serve row 2"
    );
    assert_eq!(input2.id, "q-2");

    // Row 1 must be untouched by the second claim: still pending, still
    // recorded — never re-promoted, never reset to recorded=0.
    {
        let conn = store.conn().await.unwrap();
        assert_eq!(
            input_state(&conn, seq1).await,
            (None, 1),
            "the recorded row must not be re-promoted by claim_next_queue"
        );
    }

    // Nothing genuinely pending remains: row 1 is consumed, row 2 promoted.
    assert!(
        store.claim_next_queue("s").await.unwrap().is_none(),
        "no further claimable row may exist (row 1 is recorded, row 2 promoted)"
    );
}

#[tokio::test]
async fn promote_next_queued_never_repromotes_recorded_rows() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let seq1 = store
        .admit_input(&mk_input("s", "q-1", Delivery::Queue))
        .await
        .unwrap();
    let seq2 = store
        .admit_input(&mk_input("s", "q-2", Delivery::Queue))
        .await
        .unwrap();

    assert_eq!(
        store.promote_next_queued("s").await.unwrap(),
        Some(seq1),
        "first promote serves the oldest pending row"
    );
    store.mark_inputs_recorded("s", &[seq1]).await.unwrap();
    store.unpromote_inputs("s", &[seq1]).await.unwrap();

    // The next promote must target row 2, never the consumed row 1.
    assert_eq!(
        store.promote_next_queued("s").await.unwrap(),
        Some(seq2),
        "promote_next_queued must skip the recorded=1 row and target row 2"
    );
    {
        let conn = store.conn().await.unwrap();
        assert_eq!(
            input_state(&conn, seq1).await,
            (None, 1),
            "the recorded row must not be re-promoted by promote_next_queued"
        );
    }

    // Nothing left to promote.
    assert_eq!(
        store.promote_next_queued("s").await.unwrap(),
        None,
        "no further promotable row may exist (row 1 is recorded, row 2 promoted)"
    );
}
