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
fn database(path: &Path, utf16: bool) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(path).unwrap();
    if utf16 {
        conn.execute_batch("PRAGMA encoding='UTF-16le';").unwrap();
    }
    conn.execute_batch("CREATE TABLE records(value TEXT);")
        .unwrap();
    conn
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
    for utf16 in [false, true] {
        let h = Harness::new().await;
        agent(&h).await;
        let file = resource(&h, "notes.db");
        let conn = database(&file, utf16);
        conn.execute("INSERT INTO records VALUES(?1)", ["operator-sample"])
            .unwrap();
        drop(conn);
        let before = std::fs::read(&file).unwrap();
        let blocked = h.call("DELETE", "/api/agents/sample", Value::Null).await;
        assert_eq!(blocked.0, 409, "{}", blocked.1);
        assert_eq!(
            blocked.1["details"]["references"],
            json!(["agent-file:memory/fixture/v1/notes.db"])
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);
        let conn = rusqlite::Connection::open(&file).unwrap();
        conn.execute("DELETE FROM records", ()).unwrap();
        drop(conn);
        let before = std::fs::read(&file).unwrap();
        let deleted = h.call("DELETE", "/api/agents/sample", Value::Null).await;
        assert_eq!(deleted.0, 200, "{}", deleted.1);
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert_eq!(
            std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
            1
        );
    }
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
    let conn = database(&file, false);
    conn.execute(
        "INSERT INTO records VALUES(?1)",
        [rusqlite::types::Value::Blob(vec![0xff, 0])],
    )
    .unwrap();
    drop(conn);
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
    let conn = database(&file, false);
    conn.execute_batch("PRAGMA journal_mode=WAL; INSERT INTO records VALUES('agent-sample');")
        .unwrap();
    let wal = PathBuf::from(format!("{}-wal", file.display()));
    let before_db = std::fs::read(&file).unwrap();
    let before_wal = std::fs::read(&wal).unwrap();
    let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(refused.0, 409, "{}", refused.1);
    assert_eq!(std::fs::read(&file).unwrap(), before_db);
    assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
    conn.execute("DELETE FROM records", ()).unwrap();
    let before_db = std::fs::read(&file).unwrap();
    let before_wal = std::fs::read(&wal).unwrap();
    let deleted = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(deleted.0, 200, "{}", deleted.1);
    assert_eq!(std::fs::read(&file).unwrap(), before_db);
    assert_eq!(std::fs::read(&wal).unwrap(), before_wal);
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
    let conn = database(&file, false);
    conn.execute_batch("INSERT INTO records VALUES('dag-flow'),('helper');")
        .unwrap();
    drop(conn);
    for path in ["/api/dag/defs/flow", "/api/dag/binaries/helper"] {
        let refused = h.call("DELETE", path, Value::Null).await;
        assert_eq!(refused.0, 409, "{}", refused.1);
    }
    let conn = rusqlite::Connection::open(&file).unwrap();
    conn.execute("DELETE FROM records", ()).unwrap();
    drop(conn);
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
    let conn = database(&file, false);
    conn.execute_batch("CREATE VIEW saved_view AS SELECT 'agent-sample' AS target;")
        .unwrap();
    drop(conn);
    let refused = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(refused.0, 409, "{}", refused.1);
}

#[tokio::test]
async fn incomplete_or_corrupt_wal_and_unverified_index_cannot_allow_deletion() {
    let source = tempfile::tempdir().unwrap();
    let original = source.path().join("source.db");
    let conn = database(&original, false);
    conn.execute_batch("PRAGMA journal_mode=WAL; INSERT INTO records VALUES('unrelated');")
        .unwrap();
    let wal_bytes = std::fs::read(source.path().join("source.db-wal")).unwrap();
    let shm_bytes = std::fs::read(source.path().join("source.db-shm")).unwrap();
    for (wal, shm, status) in [
        (
            wal_bytes[..wal_bytes.len() - 1].to_vec(),
            shm_bytes.clone(),
            503,
        ),
        (
            {
                let mut b = wal_bytes.clone();
                b[24] ^= 1;
                b
            },
            shm_bytes.clone(),
            503,
        ),
        (
            {
                let mut b = wal_bytes.clone();
                b[50] ^= 1;
                b
            },
            shm_bytes.clone(),
            503,
        ),
        (wal_bytes.clone(), vec![0; shm_bytes.len()], 409),
    ] {
        let h = Harness::new().await;
        agent(&h).await;
        let file = resource(&h, "notes.db");
        std::fs::copy(&original, &file).unwrap();
        std::fs::write(file.with_file_name("notes.db-wal"), wal).unwrap();
        std::fs::write(file.with_file_name("notes.db-shm"), shm).unwrap();
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
