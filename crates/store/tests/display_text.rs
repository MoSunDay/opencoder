//! `session_inputs.display_text` integration tests.
//!
//! The `display_text` column preserves the verbatim display original of a
//! submitted input (which may contain the `$skill` token) so the TUI queue/
//! steer panel can restore it after resume/reload, while `prompt` keeps the
//! clean token-stripped text that is fed to the LLM. Covers round-trip, NULL
//! fallback for old rows, the v5 -> v6 migration, the claim contract (LLM
//! only ever sees `prompt`), and bundle export/import fidelity.

use opencoder_store::{
    export_bundle, import_bundle, read_bundle, write_bundle, Delivery, LibsqlStore, SessionInput,
    SessionMeta, Store,
};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn make_session(store: &LibsqlStore, id: &str, now: i64) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    };
    store.create_session(&meta).await.unwrap();
}

/// Open a raw libsql connection to a db file, bypassing `bootstrap`, so a test
/// can hand-write an old-version schema before reopening via `LibsqlStore`.
async fn raw_open(db_path: &std::path::Path) -> libsql::Connection {
    use libsql::Builder;
    let db = Builder::new_local(db_path).build().await.unwrap();
    db.connect().unwrap()
}

fn input(
    seq: i64,
    session_id: &str,
    delivery: Delivery,
    prompt: &str,
    display_text: Option<&str>,
) -> SessionInput {
    SessionInput {
        seq: None,
        id: format!("in-{seq}"),
        session_id: session_id.to_string(),
        delivery,
        prompt: prompt.into(),
        images: Vec::new(),
        display_text: display_text.map(|d| d.to_string()),
        admitted_seq: seq,
        promoted_seq: None,
    }
}

#[tokio::test]
async fn display_text_roundtrip() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    store
        .admit_input(&input(
            1,
            "s",
            Delivery::Queue,
            "clean follow-up",
            Some("display: run $skill then report"),
        ))
        .await
        .unwrap();

    let pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].display_text.as_deref(),
        Some("display: run $skill then report"),
        "display_text must round-trip verbatim (incl. the skill token)"
    );
    assert_eq!(
        pending[0].prompt, "clean follow-up",
        "prompt must stay the clean token-stripped text"
    );
}

#[tokio::test]
async fn display_text_none_falls_back() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    // Admitted without a distinct display form -> NULL (old-row compatible).
    store
        .admit_input(&input(1, "s", Delivery::Steer, "plain steer", None))
        .await
        .unwrap();

    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].display_text, None,
        "rows admitted without display_text must read back None (prompt fallback)"
    );
    assert_eq!(pending[0].prompt, "plain steer");
}

#[tokio::test]
async fn v5_to_v6_migration_adds_display_text() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("migrate-v6.db");

    // Phase 1: hand-write a faithful v5 database. session_inputs carries the
    // full v5 shape (incl. images_json) but no display_text; sessions carries
    // task_type. On reopen only the `if from < 6` branch runs.
    {
        let conn = raw_open(&db_path).await;
        conn.execute("CREATE TABLE schema_version (version INTEGER NOT NULL)", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE TABLE sessions (\
               id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER,\
               handoff_seq INTEGER, handoff_plan TEXT, skill TEXT,\
               task_type TEXT NOT NULL DEFAULT 'parent')",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TABLE session_inputs (\
               seq INTEGER PRIMARY KEY AUTOINCREMENT,\
               id TEXT NOT NULL, session_id TEXT NOT NULL,\
               delivery TEXT NOT NULL, prompt TEXT NOT NULL,\
               images_json TEXT NOT NULL DEFAULT '[]',\
               admitted_seq INTEGER NOT NULL, promoted_seq INTEGER)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (5)", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 1, 1)",
            (),
        )
        .await
        .unwrap();
        // One pre-existing row with no display_text.
        conn.execute(
            "INSERT INTO session_inputs (id, session_id, delivery, prompt, admitted_seq)\
             VALUES ('old-1', 's1', 'queue', 'old prompt', 1)",
            (),
        )
        .await
        .unwrap();
    }

    // Reopen: bootstrap() -> migrate(conn, 5) -> if from < 6.
    let store = LibsqlStore::open(&db_path).await.unwrap();

    // The display_text column physically exists (prepare fails if absent).
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT display_text FROM session_inputs LIMIT 1")
            .await
            .expect("display_text column must exist after v6 migration");
        drop(stmt);
    }

    // Old row keeps NULL display_text.
    {
        let conn = store.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT display_text FROM session_inputs WHERE id = 'old-1'")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: Option<String> = r.get(0).unwrap();
        assert_eq!(v, None, "pre-existing rows must keep display_text NULL");
    }

    // New admits with display_text are writable and readable.
    store
        .admit_input(&input(
            2,
            "s1",
            Delivery::Queue,
            "clean new",
            Some("new $skill display"),
        ))
        .await
        .unwrap();
    let pending = store.pending_inputs("s1", Delivery::Queue).await.unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(
        pending[1].display_text.as_deref(),
        Some("new $skill display")
    );
    assert_eq!(
        pending[0].display_text, None,
        "old row keeps NULL via pending_inputs"
    );

    // Version bumped to the latest (SCHEMA_VERSION=8 after the
    // summary_images_json migration), and a second reopen is idempotent.
    drop(store);
    let store2 = LibsqlStore::open(&db_path).await.unwrap();
    {
        let conn = store2.conn().await.unwrap();
        let stmt = conn
            .prepare("SELECT version FROM schema_version LIMIT 1")
            .await
            .unwrap();
        let mut rows = stmt.query(()).await.unwrap();
        let r = rows.next().await.unwrap().unwrap();
        let v: i64 = r.get(0).unwrap();
        assert_eq!(v, 9, "schema version must be 9 (latest) after v5 migration");
    }
    let again = store2.pending_inputs("s1", Delivery::Queue).await.unwrap();
    assert_eq!(again.len(), 2, "re-open keeps data intact");
}

#[tokio::test]
async fn claim_next_queue_keeps_prompt_clean_with_display_text() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    store
        .admit_input(&input(
            1,
            "s",
            Delivery::Queue,
            "clean text",
            Some("clean text $skill token"),
        ))
        .await
        .unwrap();

    // The runner drain contract: claim returns the clean prompt (LLM input)
    // while display_text stays the verbatim original for the TUI mirror.
    let (_, claimed) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("a queued input to be claimed");
    assert_eq!(
        claimed.prompt, "clean text",
        "LLM must only ever see the clean prompt"
    );
    assert_eq!(
        claimed.display_text.as_deref(),
        Some("clean text $skill token"),
        "display_text must preserve the verbatim original on claim"
    );
}

#[tokio::test]
async fn bundle_roundtrip_preserves_display_text() {
    let (dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    // One input with a distinct display form, one old-style input with None.
    store
        .admit_input(&input(
            1,
            "s",
            Delivery::Queue,
            "clean-1",
            Some("display-1 $skill"),
        ))
        .await
        .unwrap();
    store
        .admit_input(&input(2, "s", Delivery::Queue, "clean-2", None))
        .await
        .unwrap();

    // Export -> binary -> read back -> import into a fresh store.
    let bundle = export_bundle(&store, "s").await.unwrap();
    let mut buf = Vec::new();
    write_bundle(&bundle, &mut buf).unwrap();
    let mut cursor = std::io::Cursor::new(&buf);
    let restored = read_bundle(&mut cursor).unwrap();

    let dir2 = tempfile::tempdir().unwrap();
    let store2 = LibsqlStore::open(dir2.path().join("test2.db"))
        .await
        .unwrap();
    import_bundle(&store2, &restored, None).await.unwrap();

    let pending = store2.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(
        pending[0].display_text.as_deref(),
        Some("display-1 $skill"),
        "display_text must survive bundle export/import"
    );
    assert_eq!(pending[0].prompt, "clean-1");
    assert_eq!(
        pending[1].display_text, None,
        "None display_text (serde-omitted) must round-trip as None"
    );
    assert_eq!(pending[1].prompt, "clean-2");
    let _ = dir;
}
