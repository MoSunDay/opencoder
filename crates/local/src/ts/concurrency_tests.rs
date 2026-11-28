//! ts commands beside a live session owner (a TUI in another process): the
//! owner keeps its store or registry open as a writer, and a ts command is
//! simply a second writer beside it -- nothing is refused, deferred or
//! reported as in use.
//!
//! A writer opened in this process shares its pool with every later open of
//! the same file, so the live owner is a child process running this test
//! binary (as in `crates/database/tests/processes.rs`) that opens the
//! database, marks itself holding it and keeps the handle until released.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use opencoder_store::{SessionMeta, Store, TsRecord, TsRegistry, TursoStore};

use super::super::registry::{
    open_registry_at, read_records_at, records_or_legacy_import_at, register,
};
use super::{cleanup, delete, CleanupReport};

type Child = tokio::process::Child;

/// Env contract with the child process: the database to hold, the marker it
/// writes once it holds it, the marker it waits for before letting go, and
/// its role: a (legacy v3 or current) per-workdir store, or the registry.
const HOLDER_DB: &str = "OPENCODER_TEST_TS_HOLDER_DB";
const HOLDER_HELD: &str = "OPENCODER_TEST_TS_HOLDER_HELD";
const HOLDER_RELEASE: &str = "OPENCODER_TEST_TS_HOLDER_RELEASE";
const HOLDER_ROLE: &str = "OPENCODER_TEST_TS_HOLDER_ROLE";
const ROLE_STORE: &str = "store";
const ROLE_LEGACY_STORE: &str = "legacy-store";
const ROLE_REGISTRY: &str = "registry";
/// The row the registry owner commits before reporting that it holds the
/// registry, and the file holding the ids it read once released.
const CHILD_ROW: &str = "CHILD1";
const HOLDER_SEEN: &str = "holder-seen";
/// The hand-written v3 database of `crates/store/tests/schema_v4_migration.rs`
/// (no `kind`/`task_type` on `sessions`, so a reader's list query fails on
/// it) with one ts-owned session.
const V3_SCHEMA: &str = "\
CREATE TABLE schema_version (version INTEGER NOT NULL); \
CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT, workdir_hash TEXT, \
created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, summary TEXT, summary_seq INTEGER, \
handoff_seq INTEGER, handoff_plan TEXT, skill TEXT); \
CREATE TABLE session_inputs (seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, \
session_id TEXT NOT NULL, delivery TEXT NOT NULL, prompt TEXT NOT NULL, \
admitted_seq INTEGER NOT NULL, promoted_seq INTEGER); \
INSERT INTO schema_version (version) VALUES (3); \
INSERT INTO sessions (id, created_at, updated_at) VALUES ('TSL1', 1, 1);";

/// Libtest name of `test` inside this module (without the crate prefix,
/// which is not part of the filter `--exact` matches).
fn test_path(test: &str) -> String {
    let module = module_path!();
    let (_, rest) = module.split_once("::").expect("crate-prefixed module path");
    format!("{rest}::{test}")
}

fn marker(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).expect("holder marker env"))
}

/// Sorted ids of registry rows, for order-independent comparisons.
fn record_ids(records: &[TsRecord]) -> Vec<String> {
    let mut ids: Vec<String> = records.iter().map(|record| record.id.clone()).collect();
    ids.sort();
    ids
}

/// A store session with no model (the legacy ts marker) and no messages.
fn session_meta(id: &str, created: i64) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        created_at: created,
        updated_at: created,
        ..Default::default()
    }
}

/// Create the message-less session `id` in `store`.
async fn create_session(store: &TursoStore, id: &str, created: i64) {
    store
        .create_session(&session_meta(id, created))
        .await
        .unwrap();
}

/// A registry row owned by `store_dir`: a non-empty `preview` marks a started
/// session, an empty one a never-started seed.
fn ts_record(id: &str, store_dir: &Path, created: i64, preview: &str) -> TsRecord {
    TsRecord {
        id: id.into(),
        store_dir: Some(store_dir.to_path_buf()),
        created_at: created,
        updated_at: created,
        preview: preview.into(),
        ..Default::default()
    }
}

/// Upsert `record` into the registry.
async fn upsert(registry: &TsRegistry, record: &TsRecord) {
    registry.upsert(record).await.unwrap();
}

// ===== child side ==========================================================

/// Child side of the contract: when the env names a database, play the
/// requested holder role and report `true` (the test body proper is skipped).
async fn run_holder_role() -> bool {
    let Some(db) = std::env::var_os(HOLDER_DB) else {
        return false;
    };
    let db = PathBuf::from(db);
    let held = marker(HOLDER_HELD);
    let release = marker(HOLDER_RELEASE);
    match std::env::var(HOLDER_ROLE).as_deref() {
        Ok(ROLE_STORE) => hold_store(&db, &held, &release, false).await,
        Ok(ROLE_LEGACY_STORE) => hold_store(&db, &held, &release, true).await,
        Ok(ROLE_REGISTRY) => hold_registry(&db, &held, &release).await,
        Ok(role) => panic!("unknown holder role {role}"),
        Err(error) => panic!("holder role env: {error}"),
    }
    true
}

/// Bounded wait for the parent's release marker, so a failing parent never
/// leaves the child behind.
async fn wait_for_release(release: &Path) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !release.exists() {
        assert!(Instant::now() < deadline, "never released");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Hold a per-workdir store as its writer until released; `legacy` first
/// turns it into a v3 store, which only a migrating writer open can list.
async fn hold_store(db: &Path, held: &Path, release: &Path, legacy: bool) {
    let dir = db.parent().expect("store dir");
    tokio::fs::create_dir_all(dir).await.unwrap();
    let store = TursoStore::open(db).await.expect("store writer open");
    if legacy {
        downgrade_to_v3(&store).await;
    }
    tokio::fs::write(held, b"held").await.expect("held marker");
    wait_for_release(release).await;
    drop(store);
}

/// Hold the registry as its writer with one row of its own committed; once
/// released, the ids its own connection reads go into `holder-seen`.
async fn hold_registry(db: &Path, held: &Path, release: &Path) {
    let registry = TsRegistry::open(db).await.expect("registry writer open");
    let dir = db.parent().expect("registry dir");
    upsert(&registry, &ts_record(CHILD_ROW, dir, 1, "")).await;
    tokio::fs::write(held, b"held").await.expect("held marker");
    wait_for_release(release).await;
    let rows = registry.list().await.expect("the owner lists the registry");
    let seen = held.with_file_name(HOLDER_SEEN);
    let text = record_ids(&rows).join(",");
    tokio::fs::write(seen, text).await.unwrap();
    drop(registry);
}

/// Turn the fresh store into a faithful legacy (v3) one while keeping its
/// writer open: every user table is dropped (all empty, foreign key checks
/// off on this diagnostic connection so the order does not matter; the
/// engine's internal sequence tables follow their AUTOINCREMENT table) and
/// the v3 core is written in its place.
async fn downgrade_to_v3(store: &TursoStore) {
    let conn = store.conn().await.expect("diagnostic connection");
    conn.execute("PRAGMA foreign_keys=OFF", ()).await.unwrap();
    let sql = "SELECT name FROM sqlite_master WHERE type = 'table'";
    let mut rows = conn.query(sql, ()).await.unwrap();
    let mut tables = Vec::new();
    while let Some(row) = rows.next().await.expect("next table") {
        let table = row.get::<String>(0).expect("table name");
        if !table.starts_with("sqlite_") && !table.starts_with("__turso_") {
            tables.push(table);
        }
    }
    drop(rows);
    assert!(!tables.is_empty(), "the fresh store has a schema to drop");
    for table in &tables {
        let sql = format!("DROP TABLE IF EXISTS {table}");
        conn.execute(&sql, ()).await.unwrap();
    }
    conn.execute_batch(V3_SCHEMA).await.unwrap();
}

// ===== parent side =========================================================

/// Start a child running `test` in the holder `role` against `db`, wait until
/// it reports holding the database, and return it with its release marker;
/// the child is killed if dropped unreleased, so a failing test leaves none.
async fn spawn_holder(test: &str, role: &str, db: &Path, dir: &Path) -> (Child, PathBuf) {
    let held = dir.join("holder-held");
    let release = dir.join("holder-release");
    let mut child = tokio::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test_path(test), "--nocapture"])
        .env(HOLDER_DB, db)
        .env(HOLDER_HELD, &held)
        .env(HOLDER_RELEASE, &release)
        .env(HOLDER_ROLE, role)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the holder process");
    let deadline = Instant::now() + Duration::from_secs(120);
    while !held.exists() {
        if let Some(status) = child.try_wait().expect("poll the holder") {
            panic!("the holder exited before holding the database: {status}");
        }
        assert!(Instant::now() < deadline, "never reported holding");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (child, release)
}

/// Let the child drop its handle through `release`, wait for its exit and
/// fail unless it exited cleanly.
async fn release_holder(child: &mut Child, release: &Path) {
    std::fs::write(release, b"release").expect("write the release marker");
    let waited = tokio::time::timeout(Duration::from_secs(120), child.wait()).await;
    let status = waited.expect("exit timeout").expect("wait");
    assert!(status.success(), "the holder exited cleanly: {status}");
}

/// Whether the store still holds session `id`, read through a fresh reader.
async fn session_row_exists(db: &Path, id: &str) -> bool {
    let reader = TursoStore::open_reader(db).await.unwrap();
    reader.get_session(id).await.unwrap().is_some()
}

// ===== tests ===============================================================

/// A legacy (v3) store a live session holds open: the scan lists it through
/// the migrating writer open and the registry holds its rows while the owner
/// is still alive -- nothing is deferred to a later pass.
#[tokio::test]
async fn legacy_import_migrates_a_store_a_live_writer_holds_open() {
    if run_holder_role().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let db = root.join("store").join("opencoder.db");
    let test = "legacy_import_migrates_a_store_a_live_writer_holds_open";
    let (mut holder, release) = spawn_holder(test, ROLE_LEGACY_STORE, &db, tmp.path()).await;

    let registry = open_registry_at(&root).await.unwrap();
    let records = registry.list().await.unwrap();
    assert_eq!(record_ids(&records), ["TSL1"], "imported, owner alive");
    assert_eq!(records[0].store_dir.as_deref(), db.parent(), "owning dir");
    assert!(registry.is_migrated().await.unwrap(), "the pass completed");
    // The store was migrated by the writer open: a reader lists it now.
    assert!(session_row_exists(&db, "TSL1").await, "session kept");
    drop(registry);

    release_holder(&mut holder, &release).await;
    let registry = open_registry_at(&root).await.unwrap();
    let ids = record_ids(&registry.list().await.unwrap());
    assert_eq!(ids, ["TSL1"], "the next open duplicates and drops nothing");
}

/// The registry a live session holds as a writer: a ts command registers its
/// own row beside it, both rows read back through a reader, and the owner's
/// own connection sees the ts command's row too.
#[tokio::test]
async fn registry_writes_land_beside_a_live_registry_writer() {
    if run_holder_role().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    tokio::fs::create_dir_all(&root).await.unwrap();
    let ts_db = root.join("ts.db");
    let test = "registry_writes_land_beside_a_live_registry_writer";
    let (mut holder, release) = spawn_holder(test, ROLE_REGISTRY, &ts_db, tmp.path()).await;

    let registry = open_registry_at(&root).await.unwrap();
    register(&registry, "PARENT1", &workdir).await.unwrap();
    let reader = TsRegistry::open_reader(&ts_db).await.unwrap();
    let ids = record_ids(&reader.list().await.unwrap());
    assert_eq!(ids, ["CHILD1", "PARENT1"], "both rows visible, owner alive");
    drop((registry, reader));

    release_holder(&mut holder, &release).await;
    let seen_path = tmp.path().join(HOLDER_SEEN);
    let seen = tokio::fs::read_to_string(&seen_path).await.unwrap();
    assert_eq!(seen, "CHILD1,PARENT1", "the owner reads the new row");
    let reader = TsRegistry::open_reader(&ts_db).await.unwrap();
    let ids = record_ids(&reader.list().await.unwrap());
    assert_eq!(ids, ["CHILD1", "PARENT1"], "nothing lost");
}

/// No registry yet: the flow that inspects before it mutates creates and
/// migrates the registry through the one-time legacy import, and the next
/// call finds the registry and reads its rows -- the import never runs twice.
#[tokio::test]
async fn records_without_a_registry_run_the_legacy_import_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let dir = root.join("aaaa");
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let store = TursoStore::open(dir.join("opencoder.db")).await.unwrap();
    create_session(&store, "TSR1", 1).await;
    let before = read_records_at(&root).await.unwrap();
    assert!(before.is_none(), "no registry exists yet");

    let records = records_or_legacy_import_at(&root).await.unwrap();
    assert_eq!(record_ids(&records), ["TSR1"], "the first call imports");
    let ts_db = root.join("ts.db");
    assert!(ts_db.is_file(), "the import created the registry");
    let reader = TsRegistry::open_reader(&ts_db).await.unwrap();
    assert!(reader.is_migrated().await.unwrap(), "marked migrated");

    // A second legacy session appears: the import must not run again for it.
    create_session(&store, "TSR2", 2).await;
    drop(store);
    let through_reader = read_records_at(&root).await.unwrap().expect("exists");
    assert_eq!(record_ids(&through_reader), ["TSR1"], "reader sees it");
    let again = records_or_legacy_import_at(&root).await.unwrap();
    assert_eq!(record_ids(&again), ["TSR1"], "no second import");
}

/// `ts -d` and `ts -c` against a store a live session holds open: both are
/// plain writers beside the owner -- the rows go while it is alive, read
/// back through a reader, and the report has no notion of "in use".
#[tokio::test]
async fn cleanup_and_delete_write_into_a_store_a_live_writer_holds_open() {
    if run_holder_role().await {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let store_dir = root.join("store");
    tokio::fs::create_dir_all(&store_dir).await.unwrap();
    let db = store_dir.join("opencoder.db");
    let ts_db = root.join("ts.db");

    let store = TursoStore::open(&db).await.unwrap();
    for (id, created) in [("TSD1", 1), ("TSD2", 2), ("TSD3", 3)] {
        create_session(&store, id, created).await;
    }
    drop(store);
    let registry = TsRegistry::open(&ts_db).await.unwrap();
    // One started session and two never-started seeds, all dead (no tmux).
    upsert(&registry, &ts_record("TSD1", &store_dir, 1, "started")).await;
    upsert(&registry, &ts_record("TSD2", &store_dir, 2, "")).await;
    upsert(&registry, &ts_record("TSD3", &store_dir, 3, "")).await;
    let records = registry.list().await.unwrap();
    let test = "cleanup_and_delete_write_into_a_store_a_live_writer_holds_open";
    let (mut holder, release) = spawn_holder(test, ROLE_STORE, &db, tmp.path()).await;

    // `ts -d`: the store row goes, then the registry row -- owner alive.
    delete(&[], &records, "TSD2", &registry).await.unwrap();
    let gone = registry.get("TSD2").await.unwrap().is_none();
    assert!(gone, "registry row removed");
    assert!(!session_row_exists(&db, "TSD2").await, "store row removed");
    assert!(session_row_exists(&db, "TSD1").await, "other sessions kept");

    // `ts -c`: the stopped session goes and the message-less seed is swept.
    let report = cleanup(&registry, &[]).await.unwrap();
    let counts = (report.removed_stopped, report.swept_seeds);
    assert_eq!(counts, (1, 1), "{report:?}");
    assert!(report.kept_with_messages.is_empty(), "{report:?}");
    for id in ["TSD1", "TSD3"] {
        assert!(registry.get(id).await.unwrap().is_none(), "gone: {id}");
        assert!(!session_row_exists(&db, id).await, "store row gone: {id}");
    }
    let again = cleanup(&registry, &[]).await.unwrap();
    assert_eq!(again, CleanupReport::default(), "second pass: nothing left");

    release_holder(&mut holder, &release).await;
}
