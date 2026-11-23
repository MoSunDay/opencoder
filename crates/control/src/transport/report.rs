use opencoder_core::fleet::{valid_id, ExecutionIndex, IndexReportMode, INDEX_REPORT_BATCH_SIZE};
use std::collections::HashSet;

pub(super) enum BeginDecision {
    Ignore,
    Started,
    CapturePending,
}

pub(super) struct CompleteReport {
    pub inventory: std::sync::Arc<tokio::sync::Mutex<super::socket_report::inventory::Inventory>>,
    pub report_id: u64,
    pub records: Vec<ExecutionIndex>,
    pub pending_at_begin: Option<Vec<String>>,
    pub initial: bool,
    pub negotiated: bool,
    pub base_sequence: Option<u64>,
    pub removed_ids: Vec<String>,
}

struct PartialReport {
    report_id: u64,
    records: Vec<ExecutionIndex>,
    ids: HashSet<String>,
    negotiated: bool,
    base_sequence: Option<u64>,
    removed_ids: Vec<String>,
    pending_at_begin: Option<Vec<String>>,
}

#[derive(Default)]
pub(super) struct ReportCollector {
    inventory: std::sync::Arc<tokio::sync::Mutex<super::socket_report::inventory::Inventory>>,
    latest_received: u64,
    completed_once: bool,
    committed_sequence: Option<u64>,
    active: Option<PartialReport>,
}

impl ReportCollector {
    #[cfg(test)]
    pub fn begin(&mut self, report_id: u64) -> anyhow::Result<BeginDecision> {
        self.begin_sync(report_id, false, IndexReportMode::Full, None)
    }

    pub fn begin_sync(
        &mut self,
        report_id: u64,
        negotiated: bool,
        mode: IndexReportMode,
        base_sequence: Option<u64>,
    ) -> anyhow::Result<BeginDecision> {
        match mode {
            IndexReportMode::Full => {
                anyhow::ensure!(base_sequence.is_none(), "full report cannot specify a base")
            }
            IndexReportMode::Delta => {
                anyhow::ensure!(
                    negotiated && self.completed_once,
                    "delta requires a committed full report"
                );
                anyhow::ensure!(
                    base_sequence.is_some() && base_sequence == self.committed_sequence,
                    "delta base does not match committed report"
                );
            }
        }
        if report_id == 0 {
            anyhow::bail!("index report id must be positive");
        }
        if report_id <= self.latest_received {
            return Ok(BeginDecision::Ignore);
        }
        if let Some(active) = &self.active {
            if report_id == active.report_id {
                anyhow::bail!("duplicate index report begin: {report_id}");
            }
            if report_id < active.report_id {
                return Ok(BeginDecision::Ignore);
            }
        }
        self.active = Some(PartialReport {
            report_id,
            records: Vec::new(),
            ids: HashSet::new(),
            negotiated,
            base_sequence,
            removed_ids: Vec::new(),
            pending_at_begin: None,
        });
        Ok(if self.completed_once {
            BeginDecision::Started
        } else {
            BeginDecision::CapturePending
        })
    }

    pub fn set_pending_at_begin(
        &mut self,
        report_id: u64,
        pending: Vec<String>,
    ) -> anyhow::Result<()> {
        let active = self
            .active
            .as_mut()
            .filter(|active| active.report_id == report_id)
            .ok_or_else(|| anyhow::anyhow!("index report begin was replaced"))?;
        active.pending_at_begin = Some(pending);
        Ok(())
    }

    pub fn batch(&mut self, report_id: u64, records: Vec<ExecutionIndex>) -> anyhow::Result<()> {
        if report_id <= self.latest_received {
            return Ok(());
        }
        if records.len() > INDEX_REPORT_BATCH_SIZE {
            anyhow::bail!("index report batch exceeds {INDEX_REPORT_BATCH_SIZE} records");
        }
        let Some(active) = self.active.as_mut() else {
            anyhow::bail!("index report batch without begin: {report_id}");
        };
        if report_id < active.report_id {
            return Ok(());
        }
        if report_id != active.report_id {
            anyhow::bail!("index report batch does not match begin: {report_id}");
        }
        for record in &records {
            if !active.ids.insert(record.id.clone()) {
                anyhow::bail!("duplicate execution in index report: {}", record.id);
            }
        }
        active.records.extend(records);
        Ok(())
    }

    pub fn removals(&mut self, report_id: u64, removed: Vec<String>) -> anyhow::Result<()> {
        if removed.is_empty() || report_id <= self.latest_received {
            return Ok(());
        }
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("removal without begin"))?;
        if report_id < active.report_id {
            return Ok(());
        }
        anyhow::ensure!(
            report_id == active.report_id && active.base_sequence.is_some(),
            "removals require active delta"
        );
        for id in &removed {
            anyhow::ensure!(
                valid_id(id) && active.ids.insert(id.clone()),
                "invalid or duplicate removal: {id}"
            );
        }
        active.removed_ids.extend(removed);
        Ok(())
    }

    pub fn committed(&mut self, sequence: u64) {
        self.completed_once = true;
        self.committed_sequence = Some(sequence);
    }

    pub fn end(&mut self, report_id: u64) -> anyhow::Result<Option<CompleteReport>> {
        if report_id <= self.latest_received {
            return Ok(None);
        }
        let Some(active) = self.active.as_ref() else {
            anyhow::bail!("index report end without begin: {report_id}");
        };
        if report_id < active.report_id {
            return Ok(None);
        }
        if report_id != active.report_id {
            anyhow::bail!("index report end does not match begin: {report_id}");
        }
        if !self.completed_once && active.pending_at_begin.is_none() {
            anyhow::bail!("initial index report recovery baseline missing");
        }
        let active = self.active.take().expect("active report checked above");
        let initial = !self.completed_once;
        self.latest_received = report_id;
        Ok(Some(CompleteReport {
            inventory: self.inventory.clone(),
            report_id,
            records: active.records,
            pending_at_begin: active.pending_at_begin,
            initial,
            negotiated: active.negotiated,
            base_sequence: active.base_sequence,
            removed_ids: active.removed_ids,
        }))
    }
}

#[cfg(test)]
mod tests;
