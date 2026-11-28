//! `ts -r <target>` resolution: with no registry yet, a stopped legacy session
//! named by prefix is found because the one-time import runs before the prefix
//! is resolved, while a live tmux session attaches without ever running that
//! import; a prefix the imported rows make ambiguous is still refused instead
//! of the first match being taken. The resolution is exercised through its
//! inputs (tmux sessions, read-only rows, import closure), so neither tmux nor
//! the real data root is involved.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use clap::Parser;
use opencoder_store::{SessionMeta, Store, TsRecord, TsRegistry, TursoStore};

use super::{resume_target, ResumeTarget};
use crate::ts::registry::open_registry_at;
use crate::ts::tmux::ManagedSession;
use crate::Cli;

const LEGACY_ID: &str = "01LEGACYS0001";

/// A legacy per-workdir store under `root` holding one ts session (no model,
/// the pre-registry marker) with the durable `workdir` marker beside it.
async fn legacy_store(root: &Path, id: &str, workdir: &Path) -> PathBuf {
    let store_dir = root.join(format!("store-{id}").to_lowercase());
    tokio::fs::create_dir_all(&store_dir).await.unwrap();
    let db = store_dir.join("opencoder.db");
    let store = TursoStore::open(&db).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("legacy".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    drop(store);
    tokio::fs::write(
        store_dir.join("workdir"),
        workdir.to_string_lossy().as_bytes(),
    )
    .await
    .unwrap();
    db
}

fn record(id: &str, workdir: Option<&Path>, store_dir: Option<&Path>) -> TsRecord {
    TsRecord {
        id: id.into(),
        workdir: workdir.map(Path::to_path_buf),
        store_dir: store_dir.map(Path::to_path_buf),
        created_at: 1,
        updated_at: 1,
        title: None,
        preview: String::new(),
    }
}

fn live(id: &str) -> ManagedSession {
    ManagedSession {
        name: format!("opencoder-{id}"),
        tmux_id: "$1".into(),
        created: 0,
        attached: 0,
        pane_path: String::new(),
    }
}

fn parse_cli(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap()
}

#[tokio::test]
async fn stopped_legacy_prefix_is_imported_before_it_is_resolved() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    let db = legacy_store(&root, LEGACY_ID, &workdir).await;
    let cli = parse_cli(&["opencoder", "ts", "-r", "01LEGACYS"]);
    let imported = Cell::new(false);

    let target = resume_target(&cli, "01LEGACYS", &[], None, || {
        imported.set(true);
        async { open_registry_at(&root).await?.list().await }
    })
    .await
    .unwrap();

    assert_eq!(
        target,
        ResumeTarget::Stopped {
            id: LEGACY_ID.into(),
            workdir: workdir.clone(),
        },
        "the prefix must expand to the imported row and its durable workdir"
    );
    assert!(
        imported.get(),
        "no registry: the legacy import is the only way to find the session"
    );
    let registry_path = root.join("ts.db");
    assert!(registry_path.is_file(), "the import created the registry");
    let registry = TsRegistry::open_reader(&registry_path).await.unwrap();
    let row = registry
        .get(LEGACY_ID)
        .await
        .unwrap()
        .expect("imported row");
    assert_eq!(row.store_dir.as_deref(), db.parent());
    assert_eq!(row.workdir.as_deref(), Some(workdir.as_path()));
}

#[tokio::test]
async fn ambiguous_legacy_prefix_is_still_refused_after_the_import() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    legacy_store(&root, LEGACY_ID, &workdir).await;
    legacy_store(&root, "01LEGACYS0002", &workdir).await;
    let cli = parse_cli(&["opencoder", "ts", "-r", "01LEGACYS"]);
    let imported = Cell::new(false);

    let error = resume_target(&cli, "01LEGACYS", &[], None, || {
        imported.set(true);
        async { open_registry_at(&root).await?.list().await }
    })
    .await
    .unwrap_err();

    assert!(
        imported.get(),
        "the import is what makes both candidates visible"
    );
    assert!(
        error.to_string().contains(
            "ambiguous global tmux session prefix `01LEGACYS` matches: 01LEGACYS0001, 01LEGACYS0002"
        ),
        "the re-resolution keeps the ambiguity check: {error:#}"
    );
}

#[tokio::test]
async fn unknown_target_is_reported_after_the_import_ran() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    legacy_store(&root, LEGACY_ID, &workdir).await;
    let cli = parse_cli(&["opencoder", "ts", "-r", "01MISSING0"]);
    let imported = Cell::new(false);

    let error = resume_target(&cli, "01MISSING0", &[], None, || {
        imported.set(true);
        async { open_registry_at(&root).await?.list().await }
    })
    .await
    .unwrap_err();

    assert!(
        imported.get(),
        "a stopped target is only known to be missing once everything is imported"
    );
    assert!(
        error
            .to_string()
            .contains("no global tmux session matching `01MISSING0`"),
        "{error:#}"
    );
}

#[tokio::test]
async fn live_session_attaches_without_running_the_legacy_import() {
    let cli = parse_cli(&["opencoder", "ts", "-r", "01LIVE000"]);
    let imported = Cell::new(false);

    let target = resume_target(
        &cli,
        "01LIVE000",
        &[live("01LIVE00001"), live("01OTHER00001")],
        None,
        || {
            imported.set(true);
            async { Ok(Vec::new()) }
        },
    )
    .await
    .unwrap();

    assert_eq!(
        target,
        ResumeTarget::Live("opencoder-01LIVE00001".into()),
        "a prefix naming one live session attaches to it"
    );
    assert!(
        !imported.get(),
        "attaching never takes the registry writer, not even for the import"
    );
}

#[tokio::test]
async fn registry_rows_resolve_stopped_prefixes_without_the_legacy_import() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().join("project");
    tokio::fs::create_dir_all(&workdir).await.unwrap();
    let cli = parse_cli(&["opencoder", "ts", "-r", "01REGIST0"]);
    let imported = Cell::new(false);

    let target = resume_target(
        &cli,
        "01REGIST0",
        &[],
        Some(vec![record("01REGIST0001", Some(&workdir), None)]),
        || {
            imported.set(true);
            async { Ok(Vec::new()) }
        },
    )
    .await
    .unwrap();

    assert_eq!(
        target,
        ResumeTarget::Stopped {
            id: "01REGIST0001".into(),
            workdir,
        }
    );
    assert!(!imported.get(), "an existing registry is only read");
}

#[tokio::test]
async fn explicit_workdir_must_own_the_stopped_session() {
    let tmp = tempfile::tempdir().unwrap();
    let owner = tmp.path().join("owner");
    tokio::fs::create_dir_all(&owner).await.unwrap();
    let store_dir = opencoder_core::data_dir_for(&owner);
    let records = vec![record("01OWNED00001", None, Some(&store_dir))];
    let never = || async { Ok(Vec::new()) };

    let cli = parse_cli(&[
        "opencoder",
        "--workdir",
        owner.to_str().unwrap(),
        "ts",
        "-r",
        "01OWNED0",
    ]);
    assert_eq!(
        resume_target(&cli, "01OWNED0", &[], Some(records.clone()), never)
            .await
            .unwrap(),
        ResumeTarget::Stopped {
            id: "01OWNED00001".into(),
            workdir: owner.canonicalize().unwrap(),
        },
        "an explicit --workdir that owns the session cold-starts there"
    );

    let elsewhere = tmp.path().join("elsewhere");
    tokio::fs::create_dir_all(&elsewhere).await.unwrap();
    let cli = parse_cli(&[
        "opencoder",
        "--workdir",
        elsewhere.to_str().unwrap(),
        "ts",
        "-r",
        "01OWNED0",
    ]);
    let error = resume_target(&cli, "01OWNED0", &[], Some(records), never)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("--workdir does not own global tmux session `01OWNED00001`"),
        "{error:#}"
    );
}

#[tokio::test]
async fn stopped_session_without_a_recorded_workdir_asks_for_one() {
    let cli = parse_cli(&["opencoder", "ts", "-r", "01BARE00001"]);
    let error = resume_target(
        &cli,
        "01BARE00001",
        &[],
        Some(vec![record("01BARE00001", None, None)]),
        || async { Ok(Vec::new()) },
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("predates workdir tracking; resume once with --workdir"),
        "{error:#}"
    );
}
