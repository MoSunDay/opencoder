//! A `sessions` row can carry SQL NULL in `created_at` / `updated_at` (a table
//! shape without the current NOT NULL constraints, or a damaged row). Unlike
//! the nullable `messages` metadata columns there is no sane default (a
//! fabricated stamp reorders `/task` and moves the resume boundary), so
//! decoding stays strict — but the failure must name the session and column
//! instead of surfacing as a bare "conversion failure".

use super::*;

// A `sessions` shape without the NOT NULL constraints: both stamps nullable,
// no `task_type`.
async fn seed_legacy_nullable_sessions(db_path: &std::path::Path) {
    let conn = raw_open(db_path).await;
    conn.execute(
        "CREATE TABLE sessions (\
           id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT,\
           created_at INTEGER, updated_at INTEGER, summary TEXT, summary_seq INTEGER)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, title, created_at, updated_at) VALUES ('healthy', 'h', 7, 9)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, title, created_at, updated_at) VALUES ('null-created', 'c', NULL, 9)",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, title, created_at, updated_at) VALUES ('null-updated', 'u', 7, NULL)",
        (),
    )
    .await
    .unwrap();
}

/// Every damaged stamp fails the read with the session and the column named;
/// nothing is defaulted and no row is skipped. A healthy row on the same
/// legacy shape keeps decoding exactly as before.
#[tokio::test]
async fn null_session_stamps_report_session_and_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-nullable-sessions.db");
    seed_legacy_nullable_sessions(&path).await;

    let store = TursoStore::open(&path).await.unwrap();

    let created = match store.get_session("null-created").await {
        Ok(meta) => panic!("a NULL created_at must not be defaulted: {meta:?}"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        created.contains("sessions.created_at"),
        "error names the offending column: {created}"
    );
    assert!(
        created.contains("session null-created"),
        "error names the session: {created}"
    );

    let updated = match store.get_session("null-updated").await {
        Ok(meta) => panic!("a NULL updated_at must not be defaulted: {meta:?}"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        updated.contains("sessions.updated_at"),
        "error names the offending column: {updated}"
    );
    assert!(
        updated.contains("session null-updated"),
        "error names the session: {updated}"
    );
    assert!(
        !updated.contains("sessions.created_at"),
        "the healthy column of the same row is not blamed: {updated}"
    );

    // Strictness is per row: the healthy row on the same legacy table is
    // unaffected, and a missing id still reads as `None` rather than an error.
    let healthy = store
        .get_session("healthy")
        .await
        .unwrap()
        .expect("healthy row");
    assert_eq!(healthy.id, "healthy");
    assert_eq!(healthy.created_at, 7);
    assert_eq!(healthy.updated_at, 9);
    assert!(store.get_session("absent").await.unwrap().is_none());
}
