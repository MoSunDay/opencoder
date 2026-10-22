//! A connection-local baseline advances only after the final commit acknowledgement.
use super::*;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const FULL_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Default)]
pub(super) struct DeltaState {
    committed: Option<Baseline>,
    pending: Option<Baseline>,
    last_full: Option<Instant>,
    pending_full: bool,
    source: Option<(u64, u64)>,
    pending_source: Option<(u64, u64)>,
}
struct Baseline {
    sequence: u64,
    records: BTreeMap<String, ExecutionIndex>,
}
pub(super) struct Changes {
    pub base_sequence: Option<u64>,
    pub removed_ids: Vec<String>,
}
impl DeltaState {
    pub fn source_cursor(&self) -> Option<u64> {
        if self
            .last_full
            .is_none_or(|at| at.elapsed() >= FULL_INTERVAL)
        {
            None
        } else {
            self.source.map(|(revision, _)| revision)
        }
    }
    pub fn prepare_source(
        &mut self,
        report: &PreparedReport,
        revision: u64,
        full: bool,
        removed_ids: Vec<String>,
    ) -> Result<Option<Changes>> {
        anyhow::ensure!(
            self.pending_source.is_none(),
            "source report already in flight"
        );
        let base_sequence = if full {
            None
        } else {
            Some(
                self.source
                    .context("source delta requires committed full inventory")?
                    .1,
            )
        };
        if !full && report.records.is_empty() && removed_ids.is_empty() {
            if let Some((cursor, _)) = self.source.as_mut() {
                *cursor = revision;
            }
            return Ok(None);
        }
        self.pending_source = Some((revision, report.snapshot.sequence));
        self.pending_full = full;
        Ok(Some(Changes {
            base_sequence,
            removed_ids,
        }))
    }
    /// Replaces report records with changes, retaining only the five-field indexes.
    pub fn prepare(&mut self, report: &mut PreparedReport) -> Result<Option<Changes>> {
        anyhow::ensure!(self.pending.is_none(), "index report already in flight");
        let mut current = BTreeMap::new();
        for record in std::mem::take(&mut report.records) {
            anyhow::ensure!(
                !current.contains_key(&record.id),
                "duplicate inventory execution"
            );
            current.insert(record.id.clone(), record);
        }
        let full = self.committed.is_none()
            || self
                .last_full
                .is_none_or(|time| time.elapsed() >= FULL_INTERVAL);
        let (base_sequence, removed_ids) = if full {
            report.records = current.values().cloned().collect();
            (None, Vec::new())
        } else {
            let previous = self.committed.as_ref().expect("full baseline");
            report.records = current
                .iter()
                .filter(|(id, record)| previous.records.get(*id) != Some(record))
                .map(|(_, record)| record.clone())
                .collect();
            let removed = previous
                .records
                .keys()
                .filter(|id| !current.contains_key(*id))
                .cloned()
                .collect::<Vec<_>>();
            if report.records.is_empty() && removed.is_empty() {
                return Ok(None);
            }
            (Some(previous.sequence), removed)
        };
        self.pending_full = full;
        self.pending = Some(Baseline {
            sequence: report.snapshot.sequence,
            records: current,
        });
        Ok(Some(Changes {
            base_sequence,
            removed_ids,
        }))
    }
    pub fn commit(&mut self) {
        if let Some(source) = self.pending_source.take() {
            self.source = Some(source);
        }
        self.committed = self.pending.take();
        if self.pending_full {
            self.last_full = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests;
