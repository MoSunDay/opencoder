//! Schema bootstrap under concurrent first opens: several processes open one
//! brand-new store (and one brand-new ts registry) at once. Every bootstrap
//! runs inside `BEGIN IMMEDIATE` with the bounded busy retry, so each process
//! either installs the schema or finds it installed; nobody fails on the
//! contention, and every session the children create afterwards is committed
//! exactly once.
use opencoder_store::{SessionFilter, SessionMeta, Store, TsRecord, TsRegistry, TursoStore};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CHILDREN: usize = 4;
/// Child contract: the store, the registry, the child's index and the file
/// the parent creates to start every child at once.
const CHILD_STORE: &str = "OPENCODER_TEST_BOOTSTRAP_STORE";
const CHILD_REGISTRY: &str = "OPENCODER_TEST_BOOTSTRAP_REGISTRY";
const CHILD_INDEX: &str = "OPENCODER_TEST_BOOTSTRAP_INDEX";
const CHILD_GO: &str = "OPENCODER_TEST_BOOTSTRAP_GO";

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        ..Default::default()
    }
}

/// Child role: wait for the start signal, then open both fresh databases
/// and commit into each.
async fn child(store: &Path, registry: &Path, go: &Path, index: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !go.exists() {
        assert!(
            Instant::now() < deadline,
            "the parent never started the children"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let id = format!("CHILD-{index}");
    let store_handle = TursoStore::open(store).await.unwrap();
    store_handle.create_session(&meta(&id)).await.unwrap();
    let registry_handle = TsRegistry::open(registry).await.unwrap();
    registry_handle
        .upsert(&TsRecord {
            id: id.clone(),
            store_dir: Some(store.parent().unwrap().to_path_buf()),
            ..Default::default()
        })
        .await
        .unwrap();
}

fn spawn(test: &str, store: &Path, registry: &Path, go: &Path, index: usize) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD_STORE, store)
        .env(CHILD_REGISTRY, registry)
        .env(CHILD_GO, go)
        .env(CHILD_INDEX, index.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn concurrent_first_opens_all_bootstrap_and_commit() {
    const TEST: &str = "concurrent_first_opens_all_bootstrap_and_commit";
    if let Ok(store) = std::env::var(CHILD_STORE) {
        child(
            Path::new(&store),
            &PathBuf::from(std::env::var(CHILD_REGISTRY).unwrap()),
            &PathBuf::from(std::env::var(CHILD_GO).unwrap()),
            std::env::var(CHILD_INDEX).unwrap().parse().unwrap(),
        )
        .await;
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let store = directory.path().join("store.db");
    let registry = directory.path().join("ts.db");
    let go = directory.path().join("go");
    let children: Vec<Child> = (0..CHILDREN)
        .map(|index| spawn(TEST, &store, &registry, &go, index))
        .collect();
    // Every child is waiting: release them together onto the fresh files.
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(&go, b"").unwrap();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "a concurrent first open failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let mut sessions: Vec<String> = TursoStore::open(&store)
        .await
        .unwrap()
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    sessions.sort();
    let expected: Vec<String> = (0..CHILDREN)
        .map(|index| format!("CHILD-{index}"))
        .collect();
    assert_eq!(
        sessions, expected,
        "every child committed exactly one session"
    );

    let records = TsRegistry::open(&registry)
        .await
        .unwrap()
        .list()
        .await
        .unwrap();
    let mut ids: Vec<String> = records.into_iter().map(|record| record.id).collect();
    ids.sort();
    assert_eq!(
        ids, expected,
        "every child registered exactly one ts session"
    );
}
