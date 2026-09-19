//! One-time import of the legacy `schedules.json` definitions into the
//! libsql `schedules` table (schema v27).
//!
//! Since v27 the table is the scheduler's definition source of truth and
//! the JSON file degrades to a seed input: on boot, ONLY when the table is
//! empty, every valid file entry is imported (invalid entries are warned
//! and skipped — the file was previously fail-soft too). Once definitions
//! exist, the file is dead weight for job bodies; a server restart must
//! never resurrect a definition the operator deleted, so the import is
//! strictly table-empty gated (skip-don't-merge, like `seed_review_dags`).
//! `scan_interval_secs` keeps its file role forever — see `scheduler`.

use opencoder_store::Store;
use std::{path::Path, sync::Arc};

pub async fn seed_schedules(store: &Arc<dyn Store>, workdir: &Path) {
    let existing = match store.list_schedules().await {
        Ok(existing) => existing,
        Err(e) => {
            tracing::warn!(?e, "seed schedules skipped: store.list_schedules failed");
            return;
        }
    };
    if !existing.is_empty() {
        return;
    }
    let config = opencoder_core::config::load_schedules(workdir);
    if config.schedules.is_empty() {
        return;
    }
    let now = opencoder_core::message::now_ms();
    let mut seeded = 0usize;
    for job in &config.schedules {
        // Same fail-soft contract as the old file reader: a broken entry
        // never blocks the others (nor boot).
        if let Err(error) = job.validate() {
            tracing::warn!(schedule = %job.id, %error, "seed schedule skipped: invalid");
            continue;
        }
        match store.upsert_schedule(job, now).await {
            Ok(()) => seeded += 1,
            Err(e) => tracing::warn!(schedule = %job.id, ?e, "seed schedule insert failed"),
        }
    }
    if seeded > 0 {
        tracing::info!(
            seeded,
            "schedules seeded from schedules.json (one-time import)"
        );
    }
}
