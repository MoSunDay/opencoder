//! Atomic input-id admission and v19 uniqueness migration contracts.

use libsql::params;
use opencoder_store::{Delivery, InputConflict, LibsqlStore, SessionInput, SessionMeta, Store};

async fn create_session(store: &LibsqlStore, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("idempotency".into()),
            agent: Some("act".into()),
            model: None,
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 1,
            updated_at: 1,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
}

fn input(prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: "initial-execution-1".into(),
        session_id: "session-1".into(),
        delivery: Delivery::Steer,
        prompt: prompt.into(),
        images: vec!["https://example.test/image.png".into()],
        display_text: Some("display prompt".into()),
        admitted_seq: 0,
        promoted_seq: None,
    }
}

async fn scalar(conn: &libsql::Connection, sql: &str) -> i64 {
    let stmt = conn.prepare(sql).await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get(0).unwrap()
}

async fn text_scalar(conn: &libsql::Connection, sql: &str) -> String {
    let stmt = conn.prepare(sql).await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    rows.next().await.unwrap().unwrap().get(0).unwrap()
}

#[tokio::test]
async fn twenty_concurrent_retries_create_one_row_and_return_one_seq() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inputs.db");
    let first = LibsqlStore::open(&path).await.unwrap();
    create_session(&first, "session-1").await;
    drop(first);

    let mut stores = Vec::new();
    for _ in 0..20 {
        stores.push(LibsqlStore::open(&path).await.unwrap());
    }
    let tasks: Vec<_> = stores
        .into_iter()
        .map(|store| tokio::spawn(async move { store.admit_input_once(&input("hello")).await }))
        .collect();
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.unwrap().unwrap());
    }
    assert!(outcomes.iter().all(|o| o.seq == outcomes[0].seq));
    assert_eq!(outcomes.iter().filter(|o| o.inserted).count(), 1);

    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM session_inputs").await,
        1
    );

    let retry = store.admit_input_once(&input("hello")).await.unwrap();
    assert_eq!(retry.seq, outcomes[0].seq);
    assert!(!retry.inserted);
    let mut conflicts = vec![input("different")];
    let mut changed_delivery = input("hello");
    changed_delivery.delivery = Delivery::Queue;
    conflicts.push(changed_delivery);
    let mut changed_images = input("hello");
    changed_images.images = vec!["https://example.test/other.png".into()];
    conflicts.push(changed_images);
    let mut changed_display = input("hello");
    changed_display.display_text = None;
    conflicts.push(changed_display);
    for changed in conflicts {
        let err = store.admit_input_once(&changed).await.unwrap_err();
        assert!(err.downcast_ref::<InputConflict>().is_some(), "{err:#}");
    }
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM session_inputs").await,
        1
    );
    store
        .promote_inputs("session-1", 1, Delivery::Steer)
        .await
        .unwrap();
    store
        .mark_inputs_recorded("session-1", &[outcomes[0].seq])
        .await
        .unwrap();
    let consumed_retry = store.admit_input_once(&input("hello")).await.unwrap();
    assert_eq!(consumed_retry.seq, outcomes[0].seq);
    assert!(!consumed_retry.inserted, "consumption state is not payload");
}

#[tokio::test]
async fn concurrent_different_payloads_choose_one_winner_and_reject_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflicting-inputs.db");
    let seed = LibsqlStore::open(&path).await.unwrap();
    create_session(&seed, "session-1").await;
    drop(seed);

    let stores = [
        LibsqlStore::open(&path).await.unwrap(),
        LibsqlStore::open(&path).await.unwrap(),
    ];
    let gate = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let prompts = ["first contender", "second contender"];
    let mut tasks = Vec::new();
    for (store, prompt) in stores.into_iter().zip(prompts) {
        let gate = gate.clone();
        tasks.push(tokio::spawn(async move {
            gate.wait().await;
            (prompt, store.admit_input_once(&input(prompt)).await)
        }));
    }
    gate.wait().await;

    let mut winner = None;
    let mut conflicts = 0;
    for task in tasks {
        let (prompt, result) = task.await.unwrap();
        match result {
            Ok(admitted) => {
                assert!(admitted.inserted);
                assert!(winner.replace(prompt).is_none(), "only one insert may win");
            }
            Err(error) => {
                assert!(error.downcast_ref::<InputConflict>().is_some(), "{error:#}");
                conflicts += 1;
            }
        }
    }
    assert_eq!(conflicts, 1);

    let store = LibsqlStore::open(&path).await.unwrap();
    let conn = store.conn().await.unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM session_inputs").await,
        1
    );
    assert_eq!(
        text_scalar(&conn, "SELECT prompt FROM session_inputs").await,
        winner.unwrap()
    );
}

#[tokio::test]
async fn failed_insert_rolls_back_without_an_input_or_sequence() {
    let store = LibsqlStore::open_memory().await.unwrap();
    create_session(&store, "session-1").await;
    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_initial BEFORE INSERT ON session_inputs \
         WHEN NEW.id = 'initial-execution-1' BEGIN SELECT RAISE(ABORT, 'injected'); END",
        (),
    )
    .await
    .unwrap();

    assert!(store.admit_input_once(&input("hello")).await.is_err());
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM session_inputs").await,
        0
    );
    conn.execute("DROP TRIGGER reject_initial", ())
        .await
        .unwrap();
    let admitted = store.admit_input_once(&input("hello")).await.unwrap();
    assert_eq!(admitted.seq, 1);
    assert!(admitted.inserted);
}

#[tokio::test]
async fn v19_rejects_legacy_duplicates_without_mutating_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let store = LibsqlStore::open(&path).await.unwrap();
    create_session(&store, "session-1").await;
    let conn = store.conn().await.unwrap();
    conn.execute("DROP INDEX idx_inputs_session_id", ())
        .await
        .unwrap();
    conn.execute("UPDATE schema_version SET version = 18", ())
        .await
        .unwrap();
    for admitted_seq in [1_i64, 2] {
        conn.execute(
            "INSERT INTO session_inputs \
             (id, session_id, delivery, prompt, images_json, admitted_seq, promoted_seq, display_text) \
             VALUES ('duplicate', 'session-1', 'steer', 'old', '[]', ?, NULL, NULL)",
            params![admitted_seq],
        )
        .await
        .unwrap();
    }
    drop(conn);
    drop(store);

    let err = match LibsqlStore::open(&path).await {
        Ok(_) => panic!("legacy duplicates must reject the v19 migration"),
        Err(error) => error,
    };
    assert!(
        format!("{err:#}").contains("UNIQUE constraint failed"),
        "{err:#}"
    );

    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    assert_eq!(
        scalar(&conn, "SELECT version FROM schema_version").await,
        18
    );
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM session_inputs").await,
        2
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_inputs_session_id'"
        )
        .await,
        0
    );
}
