//! One-time migration of a legacy SQLite session store into the RocksDB
//! session store (`<data_dir>/opencoder.db` → `session_store_rdb_dir`).
//!
//! Contract highlights:
//! - The destination is fixed to [`opencoder_core::session_store_rdb_dir`] of
//!   the workdir's data dir; it is never user-overridable.
//! - Extraction is read-only and strict: malformed rows abort the migration.
//! - Import goes through the public `import_bundle`, never direct KV writes.
//! - Each root is prechecked (`get_session`) so reruns are idempotent, and a
//!   partial failure rolls that root back and reports it.
//! - A non-dry-run creates `<source>.pre-rdb.bak` before any mutation, then
//!   atomically writes the non-sensitive `<source>.migrated.json` marker. A
//!   valid marker plus an existing destination short-circuits; a marker whose
//!   destination is gone is reported as stale and re-migrated; a corrupt
//!   marker is a hard error.
//!
//! Pure functions + async orchestration, no classes.

mod extract;
mod marker;
mod parse;
mod rows;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use opencoder_store::{import_bundle, Delivery, RdbStore, SessionBundle, Store};

pub use marker::{
    decide, marker_path, read_marker, write_marker, MarkerState, MigrationDecision, MigrationMarker,
};
pub use rows::{MigrationCounts, SessionForest};

/// Options for one migration run. Built from a data dir via
/// [`MigrateOptions::defaults`]; `--from`/`--backup` override source/backup.
#[derive(Debug, Clone)]
pub struct MigrateOptions {
    /// Legacy SQLite file (`<data_dir>/opencoder.db` by default).
    pub source: PathBuf,
    /// RocksDB session-store directory — always derived internally.
    pub destination: PathBuf,
    /// Backup created before any mutation (`<source>.pre-rdb.bak`).
    pub backup: PathBuf,
    /// Report only; never write destination, backup or marker.
    pub dry_run: bool,
}

impl MigrateOptions {
    /// Defaults for a data dir: source `<data_dir>/opencoder.db`, destination
    /// [`opencoder_core::session_store_rdb_dir`], backup `<source>.pre-rdb.bak`.
    pub fn defaults(data_dir: &Path) -> Self {
        let source = default_source(data_dir);
        let backup = default_backup(&source);
        MigrateOptions {
            destination: opencoder_core::session_store_rdb_dir(data_dir),
            source,
            backup,
            dry_run: false,
        }
    }
}

/// Legacy source path inside a data dir.
pub fn default_source(data_dir: &Path) -> PathBuf {
    data_dir.join("opencoder.db")
}

/// Backup path derived from a source path.
pub fn default_backup(source: &Path) -> PathBuf {
    PathBuf::from(format!("{}.pre-rdb.bak", source.display()))
}

/// Outcome of one migration run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MigrationReport {
    pub dry_run: bool,
    pub already_migrated: bool,
    pub stale_marker: bool,
    pub roots_total: u64,
    pub roots_imported: u64,
    pub roots_skipped: u64,
    pub counts: MigrationCounts,
    pub absent_tables: Vec<String>,
    pub backup: Option<PathBuf>,
    pub backup_created: bool,
    pub marker: Option<PathBuf>,
}

/// Pure text rendering of a report (the CLI prints it verbatim).
pub fn render_report(report: &MigrationReport, opts: &MigrateOptions) -> String {
    let mut out = String::new();
    out.push_str(if report.dry_run {
        "legacy session store migration (dry run)\n"
    } else {
        "legacy session store migration\n"
    });
    out.push_str(&format!("  source      : {}\n", opts.source.display()));
    out.push_str(&format!("  destination : {}\n", opts.destination.display()));
    if report.already_migrated {
        out.push_str("  status      : already migrated; nothing to do\n");
        return out;
    }
    if report.stale_marker {
        out.push_str("  status      : stale marker (recorded destination missing); re-migrating\n");
    }
    let c = &report.counts;
    out.push_str(&format!(
        "  rows        : {} sessions, {} messages, {} events, {} inputs, {} subagent tasks\n",
        c.sessions, c.messages, c.events, c.inputs, c.subagent_tasks
    ));
    if !report.absent_tables.is_empty() {
        out.push_str(&format!(
            "  note        : tables absent in source: {}\n",
            report.absent_tables.join(", ")
        ));
    }
    if report.dry_run {
        out.push_str(&format!(
            "  roots       : {} would migrate (no writes performed)\n",
            report.roots_total
        ));
        return out;
    }
    out.push_str(&format!(
        "  roots       : {} imported, {} already present, {} total\n",
        report.roots_imported, report.roots_skipped, report.roots_total
    ));
    if let Some(backup) = &report.backup {
        out.push_str(&format!(
            "  backup      : {} ({})\n",
            backup.display(),
            if report.backup_created {
                "created"
            } else {
                "existing file kept"
            }
        ));
    }
    if let Some(marker) = &report.marker {
        out.push_str(&format!("  marker      : {}\n", marker.display()));
    }
    out
}

/// Run one migration (see the module docs for the full contract).
pub async fn run(opts: &MigrateOptions) -> Result<MigrationReport> {
    let marker_file = marker_path(&opts.source);
    let state = read_marker(&marker_file)?;
    let valid = match &state {
        MarkerState::Invalid(reason) => bail!(
            "invalid migration marker at {}: {reason}; fix or remove it before retrying",
            marker_file.display()
        ),
        MarkerState::Valid(m) => Some(m.as_ref()),
        MarkerState::Absent => None,
    };

    let mut report = MigrationReport {
        dry_run: opts.dry_run,
        ..Default::default()
    };
    match decide(valid, &opts.destination) {
        MigrationDecision::AlreadyMigrated(marker) => {
            report.already_migrated = true;
            report.counts = marker.counts;
            return Ok(report);
        }
        MigrationDecision::StaleMarker(_) => report.stale_marker = true,
        MigrationDecision::Proceed => {}
    }

    let dataset = extract::extract_dataset(&opts.source).await?;
    report.absent_tables = dataset.absent_tables.clone();
    let forest = rows::assemble_roots(dataset)?;
    report.roots_total = forest.roots.len() as u64;
    report.counts = forest.counts;

    if opts.dry_run {
        // No destination writes, no backup, no marker — read-only preview.
        return Ok(report);
    }

    prepare_backup(opts, &mut report).await?;
    import_roots(opts, &forest, &mut report).await?;
    write_marker(&marker_file, opts, report.counts)?;
    report.marker = Some(marker_file);
    Ok(report)
}

/// Startup hook for the local store-open path: with no legacy file this is a
/// no-op (`Ok(None)`), so a first run stays completely normal.
pub async fn ensure_migrated(data_dir: &Path) -> Result<Option<MigrationReport>> {
    let opts = MigrateOptions::defaults(data_dir);
    if !opts.source.is_file() {
        return Ok(None);
    }
    let report = run(&opts).await?;
    Ok(Some(report))
}

/// Create the pre-migration backup unless one already exists (a rerun after a
/// crash keeps the original pre-mutation copy). Failure is fatal: the source
/// is never mutated without a backup.
async fn prepare_backup(opts: &MigrateOptions, report: &mut MigrationReport) -> Result<()> {
    report.backup = Some(opts.backup.clone());
    if opts.backup.exists() {
        report.backup_created = false;
        return Ok(());
    }
    opencoder_database::maintenance::backup(opts.source.clone(), opts.backup.clone())
        .await
        .with_context(|| {
            format!(
                "cannot create pre-migration backup {} (source {})",
                opts.backup.display(),
                opts.source.display()
            )
        })?;
    report.backup_created = true;
    Ok(())
}

/// Import every root through the public importer, prechecking presence and
/// verifying counts after each root.
async fn import_roots(
    opts: &MigrateOptions,
    forest: &SessionForest,
    report: &mut MigrationReport,
) -> Result<()> {
    if let Some(parent) = opts.destination.parent() {
        std::fs::create_dir_all(parent).context("create data dir for rdb store")?;
    }
    let store = RdbStore::open(&opts.destination)
        .await
        .with_context(|| format!("open rdb session store {}", opts.destination.display()))?;
    for bundle in &forest.roots {
        let id = bundle.meta.id.clone();
        // Per-root idempotency: an existing root is skipped whole; no partial
        // writes happen for it.
        if store.get_session(&id).await?.is_some() {
            report.roots_skipped += 1;
            tracing::info!(root = %id, "legacy root already present; skipped");
            continue;
        }
        import_bundle(&store, bundle, None)
            .await
            .with_context(|| format!("import legacy root session {id}"))?;
        if let Err(error) = verify_root(&store, bundle).await {
            // Roll the failed root back so a retry starts clean, then report.
            let _ = store.delete_session(&id).await;
            return Err(error.context(format!(
                "verification failed for legacy root session {id}; root rolled back"
            )));
        }
        report.roots_imported += 1;
    }
    Ok(())
}

/// Recursively confirm the imported counts match the bundle (messages,
/// events, pending inputs and subagent tasks for the root and every child).
async fn verify_root(store: &RdbStore, bundle: &SessionBundle) -> Result<()> {
    let id = bundle.meta.id.as_str();
    let messages = store.load_messages(id).await?.len();
    anyhow::ensure!(
        messages == bundle.messages.len(),
        "session {id}: expected {} messages, store holds {messages}",
        bundle.messages.len()
    );
    let events = store.events_after(id, 0).await?.len();
    anyhow::ensure!(
        events == bundle.events.len(),
        "session {id}: expected {} events, store holds {events}",
        bundle.events.len()
    );
    let _ = store.recover_orphan_inputs(id).await;
    let steer = store.pending_inputs(id, Delivery::Steer).await?.len();
    let queue = store.pending_inputs(id, Delivery::Queue).await?.len();
    anyhow::ensure!(
        steer + queue == bundle.inputs.len(),
        "session {id}: expected {} inputs, store holds {}",
        bundle.inputs.len(),
        steer + queue
    );
    let tasks = store.list_subagent_tasks(id).await?.len();
    anyhow::ensure!(
        tasks == bundle.subagents.len(),
        "session {id}: expected {} subagent tasks, store holds {tasks}",
        bundle.subagents.len()
    );
    for sub in &bundle.subagents {
        Box::pin(verify_root(store, &sub.child)).await?;
    }
    Ok(())
}
