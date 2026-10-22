use super::InventoryChanges;
use crate::fleet::ExecutionIndex;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct InventoryQuery {
    pub generation: Option<String>,
    pub after: Option<u64>,
    pub page: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InventoryPage {
    pub generation: String,
    pub revision: u64,
    pub full: bool,
    pub records: Vec<ExecutionIndex>,
    pub removed_ids: Vec<String>,
    pub next: Option<String>,
}

#[derive(Default)]
pub struct InventoryPages(VecDeque<Snapshot>);
struct Snapshot {
    id: String,
    created: Instant,
    generation: String,
    changes: InventoryChanges,
}

impl InventoryPages {
    pub fn begin(&mut self, generation: &str, changes: InventoryChanges) -> InventoryPage {
        self.prune();
        let snapshot = Snapshot {
            id: ulid::Ulid::new().to_string(),
            created: Instant::now(),
            generation: generation.into(),
            changes,
        };
        let page = snapshot.page(0);
        if page.next.is_some() {
            while self.0.len() >= 8 {
                self.0.pop_front();
            }
            self.0.push_back(snapshot);
        }
        page
    }
    pub fn next(&mut self, generation: &str, token: &str) -> Result<InventoryPage> {
        self.prune();
        let (id, offset) = token
            .split_once(':')
            .context("invalid inventory page cursor")?;
        let offset: usize = offset.parse().context("invalid inventory page offset")?;
        let snapshot = self
            .0
            .iter()
            .find(|s| s.id == id)
            .context("inventory page expired; restart inventory")?;
        ensure!(
            snapshot.generation == generation,
            "inventory generation changed"
        );
        ensure!(
            offset > 0 && offset.is_multiple_of(256) && offset < snapshot.len(),
            "invalid inventory page offset"
        );
        Ok(snapshot.page(offset))
    }
    fn prune(&mut self) {
        self.0
            .retain(|s| s.created.elapsed() < Duration::from_secs(120));
    }
}
impl Snapshot {
    fn len(&self) -> usize {
        self.changes.records.len() + self.changes.removed_ids.len()
    }
    fn page(&self, offset: usize) -> InventoryPage {
        let end = (offset + 256).min(self.len());
        let split = self.changes.records.len();
        InventoryPage {
            generation: self.generation.clone(),
            revision: self.changes.revision,
            full: self.changes.full,
            records: self.changes.records[offset.min(split)..end.min(split)].to_vec(),
            removed_ids: self.changes.removed_ids
                [offset.saturating_sub(split)..end.saturating_sub(split)]
                .to_vec(),
            next: (end < self.len()).then(|| format!("{}:{end}", self.id)),
        }
    }
}
