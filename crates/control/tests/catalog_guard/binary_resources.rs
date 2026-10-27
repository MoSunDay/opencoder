use super::support::{binary, spec, Harness};
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// CPython 3.11 marshal output for `value = 'agent-sample'` and `value = 'unrelated'`.
// These fixtures are inspected as data and are never executed by acceptance.
const REFERENCING_CACHE: &str = "pw0NCgAAAAAAAAAAAAAAAOMAAAAAAAAAAAAAAAABAAAAAAAAAPMKAAAAlwBkAFoAZAFTACkCegxhZ2VudC1zYW1wbGVOKQHaBXZhbHVlqQDzAAAAAPoKZml4dHVyZS5wefoIPG1vZHVsZT5yBgAAAAEAAABzDgAAAPADAQEB2AgWgAWABYAFcgQAAAA=";
const UNRELATED_CACHE: &str = "pw0NCgAAAAAAAAAAAAAAAOMAAAAAAAAAAAAAAAABAAAAAAAAAPMKAAAAlwBkAFoAZAFTACkC2gl1bnJlbGF0ZWROKQHaBXZhbHVlqQDzAAAAAPoKZml4dHVyZS5wefoIPG1vZHVsZT5yBwAAAAEAAABzDgAAAPADAQEB2AgTgAWABYAFcgUAAAA=";

fn resource(h: &Harness, file: &str) -> PathBuf {
    let root = h.dir.path().join("agents/memory/fixture/v1");
    std::fs::create_dir_all(&root).unwrap();
    root.join(file)
}
async fn open(path: &Path) -> (turso::Database, turso::Connection) {
    let database = turso::Builder::new_local(path.to_str().unwrap())
        .build()
        .await
        .unwrap();
    let connection = database.connect().unwrap();
    (database, connection)
}
async fn database(path: &Path) -> (turso::Database, turso::Connection) {
    let (database, connection) = open(path).await;
    connection
        .execute("CREATE TABLE records(value TEXT)", ())
        .await
        .unwrap();
    (database, connection)
}
/// Rewrite the header text-encoding field (offset 56) to UTF-16le. The engine
/// only reads UTF-8 databases, so such files must stay unverifiable.
fn mark_utf16(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[56] = 2;
    std::fs::write(path, bytes).unwrap();
}
/// Snapshot every file (name and bytes) in a fixture directory so tests can
/// prove the guard neither adds nor rewrites sidecars next to a source file.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}
async fn agent(h: &Harness) {
    assert_eq!(
        h.call("POST", "/api/agents", json!({"name":"sample"}))
            .await
            .0,
        201
    );
}

#[tokio::test]
async fn sqlite_resource_references_are_read_from_a_copy_and_source_is_preserved() {
    let h = Harness::new().await;
    agent(&h).await;
    let file = resource(&h, "notes.db");
    let (database, connection) = database(&file).await;
    connection
        .execute("INSERT INTO records VALUES(?1)", ["operator-sample"])
        .await
        .unwrap();
    drop(connection);
    let before = std::fs::read(&file).unwrap();
    let blocked = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(blocked.0, 409, "{}", blocked.1);
    assert_eq!(
        blocked.1["details"]["references"],
        json!(["agent-file:memory/fixture/v1/notes.db"])
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let connection = database.connect().unwrap();
    connection.execute("DELETE FROM records", ()).await.unwrap();
    drop(connection);
    // The engine keeps its committed WAL sidecar; the guard must leave every
    // file next to the source database byte-for-byte unchanged.
    let before = snapshot(file.parent().unwrap());
    let deleted = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(deleted.0, 200, "{}", deleted.1);
    assert_eq!(snapshot(file.parent().unwrap()), before);
}

#[tokio::test]
async fn utf16_databases_cannot_be_verified_and_still_refuse_deletion() {
    let h = Harness::new().await;
    agent(&h).await;
    let file = resource(&h, "notes.db");
    let (database, connection) = database(&file).await;
    connection
        .execute("INSERT INTO records VALUES(?1)", ["operator-sample"])
        .await
        .unwrap();
    drop(connection);
    drop(database);
    mark_utf16(&file);
    let before = std::fs::read(&file).unwrap();
    let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(refused.0, 503, "{}", refused.1);
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert_eq!(
        h.call("GET", "/api/agents/sample/meta", Value::Null)
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn python_cache_constants_block_references_without_blocking_unrelated_cards() {
    let h = Harness::new().await;
    agent(&h).await;
    let file = resource(&h, "compiled.pyc");
    let decode = |text| {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .unwrap()
    };
    let bytes = decode(REFERENCING_CACHE);
    std::fs::write(&file, &bytes).unwrap();
    let blocked = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(blocked.0, 409, "{}", blocked.1);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    let bytes = decode(UNRELATED_CACHE);
    std::fs::write(&file, &bytes).unwrap();
    let deleted = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(deleted.0, 200, "{}", deleted.1);
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}

#[tokio::test]
async fn invalid_or_unknown_python_cache_still_refuses_deletion() {
    let original = base64::engine::general_purpose::STANDARD
        .decode(UNRELATED_CACHE)
        .unwrap();
    for (bytes, expected) in [
        (original[..24].to_vec(), 503),
        (
            {
                let mut b = original.clone();
                b.push(0);
                b
            },
            503,
        ),
        (
            {
                let mut b = original.clone();
                b[0] = 0xa8;
                b
            },
            409,
        ),
        (
            {
                let mut b = original.clone();
                b[4] = 0x80;
                b
            },
            503,
        ),
        (
            {
                let mut b = original.clone();
                b[16] = b'N';
                b
            },
            503,
        ),
    ] {
        let h = Harness::new().await;
        agent(&h).await;
        let file = resource(&h, "compiled.pyc");
        std::fs::write(&file, &bytes).unwrap();
        let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
        assert_eq!(refused.0, expected, "{}", refused.1);
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(
            h.call("GET", "/api/agents/sample/meta", Value::Null)
                .await
                .0,
            200
        );
    }
}

#[tokio::test]
async fn unverified_database_values_and_corruption_still_refuse_deletion() {
    let h = Harness::new().await;
    agent(&h).await;
    let file = resource(&h, "notes.db");
    let (database, connection) = database(&file).await;
    connection
        .execute(
            "INSERT INTO records VALUES(?1)",
            turso::params![turso::Value::Blob(vec![0xff, 0])],
        )
        .await
        .unwrap();
    drop(connection);
    drop(database);
    for bytes in [
        std::fs::read(&file).unwrap(),
        b"SQLite format 3\0incomplete".to_vec(),
    ] {
        std::fs::write(&file, &bytes).unwrap();
        let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
        assert_eq!(refused.0, 503, "{}", refused.1);
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(
            h.call("GET", "/api/agents/sample/meta", Value::Null)
                .await
                .0,
            200
        );
    }
}

#[tokio::test]
async fn active_database_wal_is_not_silently_omitted_or_modified() {
    let h = Harness::new().await;
    agent(&h).await;
    let file = resource(&h, "notes.db");
    let (database, connection) = database(&file).await;
    let mut rows = connection
        .query("PRAGMA journal_mode=WAL", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<String>(0).unwrap(), "wal");
    drop(rows);
    connection
        .execute("INSERT INTO records VALUES('agent-sample')", ())
        .await
        .unwrap();
    let wal = PathBuf::from(format!("{}-wal", file.display()));
    let before_db = std::fs::read(&file).unwrap();
    let before_wal = std::fs::read(&wal).unwrap();
    let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(refused.0, 409, "{}", refused.1);
    assert_eq!(std::fs::read(&file).unwrap(), before_db);
    assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
    connection.execute("DELETE FROM records", ()).await.unwrap();
    let before_db = std::fs::read(&file).unwrap();
    let before_wal = std::fs::read(&wal).unwrap();
    let deleted = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(deleted.0, 200, "{}", deleted.1);
    assert_eq!(std::fs::read(&file).unwrap(), before_db);
    assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
    drop(connection);
    drop(database);
}

#[tokio::test]
async fn sqlite_resources_protect_dag_and_binary_dependencies_too() {
    let h = Harness::new().await;
    assert_eq!(h.call("POST", "/api/dag/defs", spec("flow")).await.0, 200);
    assert_eq!(
        h.call("POST", "/api/dag/binaries", binary("helper", "payload"))
            .await
            .0,
        201
    );
    let file = resource(&h, "notes.db");
    let (database, connection) = database(&file).await;
    connection
        .execute_batch("INSERT INTO records VALUES('dag-flow'),('helper');")
        .await
        .unwrap();
    drop(connection);
    drop(database);
    for path in ["/api/dag/defs/flow", "/api/dag/binaries/helper"] {
        let refused = h.call("DELETE", path, Value::Null).await;
        assert_eq!(refused.0, 409, "{}", refused.1);
    }
    let (database, connection) = open(&file).await;
    connection.execute("DELETE FROM records", ()).await.unwrap();
    drop(connection);
    drop(database);
    for path in ["/api/dag/defs/flow", "/api/dag/binaries/helper"] {
        let deleted = h.call("DELETE", path, Value::Null).await;
        assert_eq!(deleted.0, 200, "{}", deleted.1);
    }
}

#[tokio::test]
async fn sqlite_view_definitions_are_checked_without_executing_the_view() {
    let h = Harness::new().await;
    agent(&h).await;
    let file = resource(&h, "notes.db");
    let (database, connection) = database(&file).await;
    connection
        .execute_batch("CREATE VIEW saved_view AS SELECT 'agent-sample' AS target;")
        .await
        .unwrap();
    drop(connection);
    drop(database);
    let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(refused.0, 409, "{}", refused.1);
}

#[tokio::test]
async fn incomplete_or_corrupt_wal_and_unverified_index_cannot_allow_deletion() {
    let source = tempfile::tempdir().unwrap();
    let original = source.path().join("source.db");
    let (database, connection) = database(&original).await;
    let mut rows = connection
        .query("PRAGMA journal_mode=WAL", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<String>(0).unwrap(), "wal");
    drop(rows);
    connection
        .execute("INSERT INTO records VALUES('unrelated')", ())
        .await
        .unwrap();
    drop(connection);
    drop(database);
    let wal_bytes = std::fs::read(source.path().join("source.db-wal")).unwrap();
    // The engine keeps no shared-memory WAL index; a derived index file that
    // fails its own header checks stays an unverified input.
    let index_bytes = vec![0u8; 32768];
    for (wal, shm, status) in [
        (wal_bytes[..wal_bytes.len() - 1].to_vec(), None, 503),
        (
            {
                let mut b = wal_bytes.clone();
                b[24] ^= 1;
                b
            },
            None,
            503,
        ),
        (
            {
                let mut b = wal_bytes.clone();
                b[50] ^= 1;
                b
            },
            None,
            503,
        ),
        (wal_bytes.clone(), Some(index_bytes), 409),
    ] {
        let h = Harness::new().await;
        agent(&h).await;
        let file = resource(&h, "notes.db");
        std::fs::copy(&original, &file).unwrap();
        std::fs::write(file.with_file_name("notes.db-wal"), wal).unwrap();
        if let Some(shm) = shm {
            std::fs::write(file.with_file_name("notes.db-shm"), shm).unwrap();
        }
        let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
        assert_eq!(refused.0, status, "{}", refused.1);
        assert_eq!(
            h.call("GET", "/api/agents/sample/meta", Value::Null)
                .await
                .0,
            200
        );
    }
}
