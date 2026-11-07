//! Integration tests for `opencoder session migrate-legacy`: end-to-end
//! migration of a legacy SQLite store into the rdb session store, marker /
//! backup semantics, dry-run isolation, corrupt-store rejection, and the
//! startup hook.

#[path = "legacy_migration/support.rs"]
mod support;

use std::path::Path;

use clap::Parser;
use opencoder_local::legacy::{self, marker_path, read_marker, MarkerState, MigrateOptions};
use opencoder_local::{Cli, Command, SessionSub};
use opencoder_store::{Delivery, RdbStore, Store, SubagentStatus};

use support::{expected_counts, ALPHA, BETA, GAMMA};

/// Options for an isolated fixture dir: source `<dir>/opencoder.db`,
/// destination `<dir>/opencoder.db.rdb`, default backup name.
fn opts_for(dir: &Path) -> MigrateOptions {
    MigrateOptions {
        source: dir.join(support::SOURCE_NAME),
        destination: dir.join("opencoder.db.rdb"),
        backup: legacy::default_backup(&dir.join(support::SOURCE_NAME)),
        dry_run: false,
    }
}

/// Full `{:#}` chain of an error, for substring assertions.
fn chain(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

fn migrate_variant(cli: &Cli) -> &SessionSub {
    match &cli.command {
        Some(Command::Session { sub }) => sub,
        other => panic!("expected session command, got {other:?}"),
    }
}

#[test]
fn cli_flags_parse_with_defaults_and_overrides() {
    let cli = Cli::parse_from(["opencoder", "session", "migrate-legacy"]);
    match migrate_variant(&cli) {
        SessionSub::MigrateLegacy {
            from,
            backup,
            dry_run,
        } => {
            assert_eq!(*from, None, "no --from defaults to <data_dir>/opencoder.db");
            assert_eq!(*backup, None);
            assert!(!*dry_run);
        }
        other => panic!("expected migrate-legacy, got {other:?}"),
    }
    let cli = Cli::parse_from([
        "opencoder",
        "--workdir",
        "/tmp/wd",
        "session",
        "migrate-legacy",
        "--from",
        "/tmp/old.db",
        "--backup",
        "/tmp/old.bak",
        "--dry-run",
    ]);
    assert_eq!(cli.workdir.as_deref(), Some(Path::new("/tmp/wd")));
    match migrate_variant(&cli) {
        SessionSub::MigrateLegacy {
            from,
            backup,
            dry_run,
        } => {
            assert_eq!(from.as_deref(), Some(Path::new("/tmp/old.db")));
            assert_eq!(backup.as_deref(), Some(Path::new("/tmp/old.bak")));
            assert!(*dry_run);
        }
        other => panic!("expected migrate-legacy, got {other:?}"),
    }
}

#[tokio::test]
async fn migrates_roots_subagent_marker_and_backup_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    support::seed(&opts.source).await.unwrap();

    let report = legacy::run(&opts).await.unwrap();
    assert!(!report.already_migrated && !report.stale_marker && !report.dry_run);
    assert_eq!(report.counts, expected_counts());
    assert_eq!(report.roots_total, 2);
    assert_eq!(report.roots_imported, 2);
    assert_eq!(report.roots_skipped, 0);
    assert!(report.backup_created, "first run creates the backup");
    assert!(opts.backup.is_file(), "backup file exists");

    // Marker: valid, format 1, exact paths and counts.
    let marker_file = marker_path(&opts.source);
    let marker = match read_marker(&marker_file).unwrap() {
        MarkerState::Valid(marker) => *marker,
        state => panic!("expected valid marker, got {state:?}"),
    };
    assert_eq!(marker.format, 1);
    assert_eq!(marker.source, opts.source.to_string_lossy());
    assert_eq!(marker.destination, opts.destination.to_string_lossy());
    assert_eq!(marker.backup, opts.backup.to_string_lossy());
    assert_eq!(marker.counts, expected_counts());
    assert!(marker.migrated_at > 0);

    // Destination content, verified through the public store API.
    let store = RdbStore::open(&opts.destination).await.unwrap();
    let alpha = store.get_session(ALPHA).await.unwrap().expect("alpha");
    assert_eq!(alpha.title.as_deref(), Some("Alpha root"));
    assert_eq!(
        alpha.workdir_hash.as_deref(),
        Some("deadbeef01"),
        "legacy workdir_hash must be preserved"
    );
    let alpha_messages = store.load_messages(ALPHA).await.unwrap();
    assert_eq!(alpha_messages.len(), 2);
    assert_eq!(alpha_messages[0].text(), "hello");
    assert_eq!(alpha_messages[1].text(), "world");
    assert_eq!(
        store.events_after(ALPHA, 0).await.unwrap().len(),
        2,
        "alpha events"
    );
    store.recover_orphan_inputs(ALPHA).await.unwrap();
    assert_eq!(
        store
            .pending_inputs(ALPHA, Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .pending_inputs(ALPHA, Delivery::Queue)
            .await
            .unwrap()
            .len(),
        1
    );
    let tasks = store.list_subagent_tasks(ALPHA).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].child_session_id, GAMMA);
    assert_eq!(tasks[0].status, SubagentStatus::Completed);
    assert_eq!(tasks[0].ok, Some(true));
    assert_eq!(tasks[0].result.as_deref(), Some("child finished"));

    let beta = store.get_session(BETA).await.unwrap().expect("beta");
    // `import_bundle` resets compaction/handoff markers by design (source
    // message seqs dangle after re-import; resume recomputes them).
    assert_eq!(beta.summary, None);
    assert_eq!(beta.summary_seq, None);
    assert_eq!(beta.handoff_seq, None);
    assert_eq!(beta.handoff_plan, None);
    assert_eq!(beta.skill.as_deref(), Some("skill-x"));
    assert_eq!(beta.autopilot_mode.as_deref(), Some("ap"));
    let beta_messages = store.load_messages(BETA).await.unwrap();
    assert_eq!(beta_messages.len(), 1, "summary=1 row must be excluded");
    assert_eq!(beta_messages[0].text(), "beta prompt");

    let gamma = store.get_session(GAMMA).await.unwrap().expect("gamma");
    assert_eq!(gamma.task_type.as_deref(), Some("subagent"));
    assert!(
        gamma.summary_images.is_empty(),
        "compaction images reset with the other markers"
    );
    assert_eq!(store.load_messages(GAMMA).await.unwrap().len(), 1);
    // The legacy source itself is never mutated or moved.
    assert!(opts.source.is_file());
}

#[tokio::test]
async fn rerun_after_success_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    support::seed(&opts.source).await.unwrap();
    legacy::run(&opts).await.unwrap();

    let report = legacy::run(&opts).await.unwrap();
    assert!(report.already_migrated, "valid marker + live destination");
    assert_eq!(report.roots_imported, 0);
    assert_eq!(report.counts, expected_counts(), "counts come from marker");

    let store = RdbStore::open(&opts.destination).await.unwrap();
    assert_eq!(store.load_messages(ALPHA).await.unwrap().len(), 2);
    assert_eq!(store.get_session(GAMMA).await.unwrap().unwrap().id, GAMMA);
}

#[tokio::test]
async fn dry_run_performs_no_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = opts_for(dir.path());
    opts.dry_run = true;
    support::seed(&opts.source).await.unwrap();

    let report = legacy::run(&opts).await.unwrap();
    assert!(report.dry_run);
    assert!(!report.already_migrated);
    assert_eq!(report.counts, expected_counts(), "extraction still runs");
    assert_eq!(report.roots_total, 2);
    assert!(!opts.destination.exists(), "no destination writes");
    assert!(!opts.backup.exists(), "no backup");
    assert!(!marker_path(&opts.source).exists(), "no marker");
}

#[tokio::test]
async fn stale_marker_re_migrates_and_keeps_original_backup() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    support::seed(&opts.source).await.unwrap();
    legacy::run(&opts).await.unwrap();
    std::fs::remove_dir_all(&opts.destination).unwrap();

    let report = legacy::run(&opts).await.unwrap();
    assert!(
        report.stale_marker,
        "marker points at a deleted destination"
    );
    assert_eq!(report.roots_imported, 2);
    assert!(
        !report.backup_created,
        "the pre-mutation backup is kept, not replaced"
    );

    let store = RdbStore::open(&opts.destination).await.unwrap();
    assert!(store.get_session(ALPHA).await.unwrap().is_some());
    assert!(matches!(
        read_marker(&marker_path(&opts.source)).unwrap(),
        MarkerState::Valid(_)
    ));
}

#[tokio::test]
async fn corrupt_marker_aborts_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    support::seed(&opts.source).await.unwrap();
    std::fs::write(marker_path(&opts.source), b"{not-json").unwrap();

    let error = legacy::run(&opts).await.unwrap_err();
    let chain = chain(&error);
    assert!(chain.contains("invalid migration marker"), "got: {chain}");
    assert!(chain.contains("corrupt marker JSON"), "got: {chain}");
    assert!(!opts.destination.exists(), "nothing imported");
    assert!(!opts.backup.exists(), "no backup either");
}

#[tokio::test]
async fn unknown_enum_cell_aborts_the_whole_migration() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    support::seed_with(
        &opts.source,
        "UPDATE messages SET role='wizard' WHERE id='m-a1';",
    )
    .await
    .unwrap();

    let error = legacy::run(&opts).await.unwrap_err();
    let chain = chain(&error);
    assert!(
        chain.contains("unknown legacy message role `wizard`"),
        "got: {chain}"
    );
    assert!(!opts.destination.exists());
    assert!(!marker_path(&opts.source).exists());
}

#[tokio::test]
async fn dangling_child_reference_aborts_instead_of_dropping_rows() {
    let dir = tempfile::tempdir().unwrap();
    let opts = opts_for(dir.path());
    support::seed_orphan_task(&opts.source).await.unwrap();

    let error = legacy::run(&opts).await.unwrap_err();
    let chain = chain(&error);
    assert!(
        chain.contains("missing child session sess-ghost"),
        "got: {chain}"
    );
    assert!(!opts.destination.exists());
    assert!(!marker_path(&opts.source).exists());
}

#[tokio::test]
async fn startup_hook_is_a_noop_without_a_legacy_file() {
    let dir = tempfile::tempdir().unwrap();
    let report = legacy::ensure_migrated(dir.path()).await.unwrap();
    assert_eq!(report, None, "no legacy file → nothing to do");
    assert!(
        dir.path().read_dir().unwrap().next().is_none(),
        "no files created"
    );
}

#[tokio::test]
async fn startup_hook_migrates_and_the_store_opens_with_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path();
    support::seed(&data_dir.join(support::SOURCE_NAME))
        .await
        .unwrap();

    let report = legacy::ensure_migrated(data_dir)
        .await
        .unwrap()
        .expect("legacy file present → migration runs");
    assert_eq!(report.roots_imported, 2);
    assert_eq!(report.counts, expected_counts());

    // The exact store `open_store` would open for this data dir.
    let store = RdbStore::open(opencoder_core::session_store_rdb_dir(data_dir))
        .await
        .unwrap();
    let alpha = store.get_session(ALPHA).await.unwrap().expect("alpha");
    assert_eq!(alpha.title.as_deref(), Some("Alpha root"));
    assert!(matches!(
        read_marker(&marker_path(&data_dir.join(support::SOURCE_NAME))).unwrap(),
        MarkerState::Valid(_)
    ));
}

/// `session_dispatch` wiring test. `data_dir_for(workdir)` resolves through
/// the platform data-local dir, so XDG_DATA_HOME is redirected for the
/// duration (this is the only test in this binary reading it) and restored.
#[tokio::test]
async fn cli_dispatch_migrates_for_the_current_workdir() {
    let _lock = env_home_lock().await;
    let home = tempfile::tempdir().unwrap();
    let workdir = tempfile::tempdir().unwrap();
    let legacy_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", home.path());
    let data_dir = opencoder_core::data_dir_for(workdir.path());
    let source = legacy_dir.path().join(support::SOURCE_NAME);
    support::seed(&source).await.unwrap();
    let cli = Cli::parse_from([
        "opencoder",
        "--workdir",
        workdir.path().to_str().unwrap(),
        "session",
        "migrate-legacy",
        "--from",
        source.to_str().unwrap(),
    ]);
    let sub = migrate_variant(&cli);
    opencoder_local::session_cmd::session_dispatch(sub, &cli)
        .await
        .unwrap();
    std::env::remove_var("XDG_DATA_HOME");

    let destination = opencoder_core::session_store_rdb_dir(&data_dir);
    let store = RdbStore::open(&destination).await.unwrap();
    assert!(store.get_session(ALPHA).await.unwrap().is_some());
    assert!(store.get_session(GAMMA).await.unwrap().is_some());
    assert!(marker_path(&source).is_file());
    assert!(legacy::default_backup(&source).is_file());
}

use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};

/// Serializes the XDG_DATA_HOME-mutating test against itself (one caller at
/// a time; no other test in this binary resolves the platform data dir).
/// Async-aware so the guard may span the dispatch awaits.
async fn env_home_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

/// The shared store-open sequence (CLI and TUI boot): with a legacy SQLite
/// store present it migrates first, then opens `opencoder.db.rdb` — the TUI
/// regression where the legacy plain-file stem was opened directly and
/// failed with `File exists (os error 17)`.
#[tokio::test]
async fn open_store_at_migrates_legacy_and_opens_the_rdb_store() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path();
    support::seed(&data_dir.join(support::SOURCE_NAME))
        .await
        .unwrap();

    let store = opencoder_local::session_cmd::open_store_at(data_dir)
        .await
        .unwrap();

    // The legacy file survives untouched beside the new rdb store.
    assert!(data_dir.join(support::SOURCE_NAME).is_file());
    assert!(data_dir.join("opencoder.db.rdb").is_dir());
    let alpha = store
        .get_session(ALPHA)
        .await
        .unwrap()
        .expect("alpha migrated");
    assert_eq!(alpha.title.as_deref(), Some("Alpha root"));

    // A second open is an idempotent already-migrated short-circuit.
    opencoder_local::session_cmd::open_store_at(data_dir)
        .await
        .unwrap();
}
