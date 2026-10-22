//! Only five-field indexes are retained; ordinary commits touch changed IDs.
use super::super::report::CompleteReport;
use opencoder_core::fleet::ExecutionIndex;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Inventory {
    sequence: Option<u64>,
    records: BTreeMap<String, ExecutionIndex>,
}

impl Inventory {
    pub fn snapshot(&self, report: &CompleteReport) -> anyhow::Result<Vec<ExecutionIndex>> {
        if report.base_sequence.is_none() {
            return Ok(report.records.clone());
        }
        anyhow::ensure!(
            self.sequence == report.base_sequence,
            "complete reconstruction requires this connection's committed baseline"
        );
        let mut records = self.records.clone();
        for id in &report.removed_ids {
            records.remove(id);
        }
        for record in &report.records {
            records.insert(record.id.clone(), record.clone());
        }
        Ok(records.into_values().collect())
    }

    pub fn commit(&mut self, report: &CompleteReport, sequence: u64) {
        if report.base_sequence.is_none() {
            self.records.clear();
        }
        for id in &report.removed_ids {
            self.records.remove(id);
        }
        for record in &report.records {
            self.records.insert(record.id.clone(), record.clone());
        }
        self.sequence = Some(sequence);
    }
}
