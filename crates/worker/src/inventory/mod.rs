//! Incremental inventory assembled from journal, database and live-loop changes.
mod derive;
mod metrics;
mod refresh;
use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::{inventory::*, ExecutionIndex};
use opencoder_store::inventory::{ProjectIndexSource, SessionIndexSource};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct InventoryCache {
    metrics: metrics::RefreshMetrics,
    log: InventoryLog,
    pages: InventoryPages,
    db_revision: Option<u64>,
    journal_revision: Option<u64>,
    checked: Option<std::time::Instant>,
    roots: BTreeMap<String, ExecutionIndex>,
    targets: BTreeMap<String, String>,
    roots_by_todo: BTreeMap<String, BTreeSet<String>>,
    sessions: BTreeMap<String, SessionIndexSource>,
    runs: BTreeMap<String, ProjectIndexSource>,
    runs_by_todo: BTreeMap<String, BTreeSet<String>>,
    running_by_todo: BTreeMap<String, BTreeSet<String>>,
    active_loops: BTreeSet<String>,
    active_roots: BTreeSet<String>,
}

impl Worker {
    pub async fn inventory_changes(&self, query: InventoryQuery) -> Result<InventoryPage> {
        let mut cache = self.inner.inventory.lock().await;
        if let Some(page) = query.page {
            return cache.pages.next(&self.inner.generation, &page);
        }
        cache.refresh(self).await?;
        let after = if query.generation.as_deref() == Some(&self.inner.generation) {
            query.after
        } else {
            None
        };
        let changes = cache.log.read(after);
        Ok(cache.pages.begin(&self.inner.generation, changes))
    }
    pub(crate) async fn cached_indexes(&self) -> Result<Vec<ExecutionIndex>> {
        let mut cache = self.inner.inventory.lock().await;
        cache.refresh(self).await?;
        Ok(cache.log.read(None).records)
    }
}
