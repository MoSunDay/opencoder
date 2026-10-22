//! Bounded change history for five-field inventories. Readers own their cursors.
mod pages;
use super::ExecutionIndex;
pub use pages::{InventoryPage, InventoryPages, InventoryQuery};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

const HISTORY_LIMIT: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryChanges {
    pub revision: u64,
    pub full: bool,
    pub records: Vec<ExecutionIndex>,
    pub removed_ids: Vec<String>,
}

#[derive(Default)]
pub struct InventoryLog {
    records: BTreeMap<String, ExecutionIndex>,
    revision: u64,
    floor: u64,
    changes: VecDeque<(u64, String)>,
}

impl InventoryLog {
    pub fn set(&mut self, record: ExecutionIndex) -> bool {
        if self.records.get(&record.id) == Some(&record) {
            return false;
        }
        self.changed(record.id.clone());
        self.records.insert(record.id.clone(), record);
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        if self.records.remove(id).is_none() {
            return false;
        }
        self.changed(id.into());
        true
    }

    fn changed(&mut self, id: String) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("inventory revision overflow");
        self.changes.push_back((self.revision, id));
        while self.changes.len() > HISTORY_LIMIT {
            self.floor = self.changes.pop_front().expect("history entry").0;
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn read(&self, after: Option<u64>) -> InventoryChanges {
        let full = after.is_none_or(|seq| seq < self.floor || seq > self.revision);
        let mut result = InventoryChanges {
            revision: self.revision,
            full,
            records: Vec::new(),
            removed_ids: Vec::new(),
        };
        if full {
            result.records.extend(self.records.values().cloned());
        } else {
            let after = after.expect("incremental cursor");
            // Walk backwards: an unchanged poll visits no history entries.
            let ids: std::collections::BTreeSet<_> = self
                .changes
                .iter()
                .rev()
                .take_while(|(seq, _)| *seq > after)
                .map(|(_, id)| id)
                .collect();
            for id in ids {
                if let Some(record) = self.records.get(id) {
                    result.records.push(record.clone());
                } else {
                    result.removed_ids.push(id.clone());
                }
            }
        }
        result
    }

    pub fn reset(&mut self, records: impl IntoIterator<Item = ExecutionIndex>) {
        let incoming: BTreeMap<_, _> = records.into_iter().map(|r| (r.id.clone(), r)).collect();
        let removed: Vec<_> = self
            .records
            .keys()
            .filter(|id| !incoming.contains_key(*id))
            .cloned()
            .collect();
        for id in removed {
            self.remove(&id);
        }
        for record in incoming.into_values() {
            self.set(record);
        }
    }
}

#[cfg(test)]
mod tests;
