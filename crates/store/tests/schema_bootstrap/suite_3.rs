//! A `messages` table created or rewritten without the current NOT NULL
//! constraints can carry SQL NULL in the metadata columns `usage_json` /
//! `synthetic`. Decoding must fall back to the documented defaults (empty
//! usage, `synthetic = false`) so Resume/Fork can load the session, while
//! content columns stay strict and name the offending
//! session/message/column.

use super::*;

// A `messages` shape without the NOT NULL constraints: every metadata column
// is nullable. The sessions table is the pre-versioning shape from
// `seed_legacy_db`.
async fn seed_legacy_nullable_messages(db_path: &std::path::Path) {
    let conn = raw_open(db_path).await;
    conn.execute(
        "CREATE TABLE sessions (\
           id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
           created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "CREATE TABLE messages (\
           seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, session_id TEXT NOT NULL,\
           role TEXT NOT NULL, agent TEXT, model TEXT, blocks_json TEXT, usage_json TEXT,\
           created_at INTEGER NOT NULL, synthetic INTEGER, display TEXT)",
        (),
    )
    .await
    .unwrap();
    for session_id in ["legacy", "corrupt"] {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES (?1, 1, 1)",
            turso::params![session_id],
        )
        .await
        .unwrap();
    }
    conn.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic, display)\
         VALUES ('legacy-normal', 'legacy', 'assistant', '[]', '{\"input_tokens\":3,\"output_tokens\":5,\"total_tokens\":8}', 11, 1, 'shown')",
        (),
    )
    .await
    .unwrap();
    // Legacy NULL in both nullable columns: usage must decode as the default
    // usage and synthetic as false, without dropping the message.
    conn.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic)\
         VALUES ('legacy-null', 'legacy', 'user', '[]', NULL, 12, NULL)",
        (),
    )
    .await
    .unwrap();
    // Content corruption on a separate session: a NULL blocks_json must stay
    // a hard error instead of being silently skipped.
    conn.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic)\
         VALUES ('corrupt-null-blocks', 'corrupt', 'user', NULL, '{}', 13, 0)",
        (),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn legacy_nullable_usage_and_synthetic_decode_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-nullable-messages.db");
    seed_legacy_nullable_messages(&path).await;

    let store = TursoStore::open(&path).await.unwrap();

    let all = store.load_messages("legacy").await.unwrap();
    assert_eq!(
        all.len(),
        2,
        "a NULL metadata column must not drop a message"
    );
    assert_eq!(all[0].id, "legacy-normal");
    assert_eq!(all[0].usage.input_tokens, 3);
    assert_eq!(all[0].usage.output_tokens, 5);
    assert_eq!(all[0].usage.total_tokens, 8);
    assert_eq!(all[0].created_at, 11);
    assert!(all[0].synthetic, "a stored synthetic flag keeps its value");
    assert_eq!(
        all[0].display.as_deref(),
        Some("shown"),
        "normal rows decode exactly as before"
    );

    assert_eq!(all[1].id, "legacy-null");
    assert_eq!(all[1].usage.input_tokens, 0);
    assert_eq!(all[1].usage.output_tokens, 0);
    assert_eq!(all[1].usage.total_tokens, 0);
    assert_eq!(all[1].created_at, 12);
    assert!(!all[1].synthetic, "NULL synthetic decodes as false");
    assert!(all[1].blocks.is_empty());

    // The OFFSET path (Resume/Fork tail load) shares the same row decoder.
    let tail = store.load_messages_after("legacy", 1).await.unwrap();
    assert_eq!(tail.len(), 1, "skip=1 keeps only the NULL-metadata row");
    assert_eq!(tail[0].id, "legacy-null");
    assert!(!tail[0].synthetic);
    assert_eq!(tail[0].usage.total_tokens, 0);

    let from_start = store.load_messages_after("legacy", 0).await.unwrap();
    assert_eq!(
        from_start.len(),
        2,
        "skip=0 returns the same rows as load_messages"
    );
    assert_eq!(from_start[0].usage.total_tokens, 8);
    assert!(!from_start[1].synthetic);
}

#[tokio::test]
async fn strict_content_columns_report_session_message_and_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-corrupt-messages.db");
    seed_legacy_nullable_messages(&path).await;

    let store = TursoStore::open(&path).await.unwrap();

    let error = match store.load_messages("corrupt").await {
        Ok(_) => panic!("a NULL blocks_json must not be fabricated or skipped"),
        Err(error) => error,
    };
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("messages.blocks_json"),
        "error names the offending column: {rendered}"
    );
    assert!(
        rendered.contains("session corrupt"),
        "error names the session: {rendered}"
    );
    assert!(
        rendered.contains("message corrupt-null-blocks"),
        "error names the message: {rendered}"
    );

    let tail_error = match store.load_messages_after("corrupt", 0).await {
        Ok(_) => panic!("the OFFSET path must fail the same way"),
        Err(error) => error,
    };
    assert!(
        format!("{tail_error:#}").contains("messages.blocks_json"),
        "OFFSET path reports the same column"
    );
}
