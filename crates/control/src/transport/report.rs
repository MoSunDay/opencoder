use opencoder_core::fleet::{ExecutionIndex, INDEX_REPORT_BATCH_SIZE};
use std::collections::HashSet;

pub(super) enum BeginDecision {
    Ignore,
    Started,
    CapturePending,
}

pub(super) struct CompleteReport {
    pub report_id: u64,
    pub records: Vec<ExecutionIndex>,
    pub pending_at_begin: Option<Vec<String>>,
    pub initial: bool,
}

struct PartialReport {
    report_id: u64,
    records: Vec<ExecutionIndex>,
    ids: HashSet<String>,
    pending_at_begin: Option<Vec<String>>,
}

#[derive(Default)]
pub(super) struct ReportCollector {
    latest_complete: u64,
    completed_once: bool,
    active: Option<PartialReport>,
}

impl ReportCollector {
    pub fn begin(&mut self, report_id: u64) -> anyhow::Result<BeginDecision> {
        if report_id == 0 {
            anyhow::bail!("index report id must be positive");
        }
        if report_id <= self.latest_complete {
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
        if report_id <= self.latest_complete {
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

    pub fn end(&mut self, report_id: u64) -> anyhow::Result<Option<CompleteReport>> {
        if report_id <= self.latest_complete {
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
        self.latest_complete = report_id;
        self.completed_once = true;
        Ok(Some(CompleteReport {
            report_id,
            records: active.records,
            pending_at_begin: active.pending_at_begin,
            initial,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};

    fn record(n: usize) -> ExecutionIndex {
        ExecutionIndex {
            id: format!("agent-{n}"),
            created_at: n as i64,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status: ExecutionStatus::Idle,
        }
    }

    #[test]
    fn empty_initial_report_completes_with_recovery_snapshot() {
        let mut collector = ReportCollector::default();
        assert!(matches!(
            collector.begin(1).unwrap(),
            BeginDecision::CapturePending
        ));
        collector
            .set_pending_at_begin(1, vec!["agent-missing".into()])
            .unwrap();
        let report = collector.end(1).unwrap().unwrap();
        assert!(report.initial);
        assert!(report.records.is_empty());
        assert_eq!(report.pending_at_begin.unwrap(), ["agent-missing"]);
    }

    #[test]
    fn multiple_batches_form_one_complete_report() {
        let mut collector = ReportCollector::default();
        collector.begin(1).unwrap();
        collector.set_pending_at_begin(1, vec![]).unwrap();
        collector.batch(1, (0..256).map(record).collect()).unwrap();
        collector.batch(1, vec![record(256)]).unwrap();
        let first = collector.end(1).unwrap().unwrap();
        assert!(first.initial);
        assert_eq!(first.records.len(), 257);
        collector.begin(2).unwrap();
        assert!(!collector.end(2).unwrap().unwrap().initial);
    }

    #[test]
    fn newer_begin_discards_half_report_and_old_complete_is_ignored() {
        let mut collector = ReportCollector::default();
        collector.begin(1).unwrap();
        collector.batch(1, vec![record(1)]).unwrap();
        collector.begin(2).unwrap();
        collector.set_pending_at_begin(2, vec![]).unwrap();
        collector.batch(1, vec![record(2)]).unwrap();
        collector.batch(2, vec![record(3)]).unwrap();
        assert!(collector.end(1).unwrap().is_none());
        assert_eq!(collector.end(2).unwrap().unwrap().records[0].id, "agent-3");
        assert!(matches!(collector.begin(1).unwrap(), BeginDecision::Ignore));
        assert!(collector.end(1).unwrap().is_none());
    }

    #[test]
    fn malformed_sequence_and_duplicate_ids_fail_closed() {
        let mut collector = ReportCollector::default();
        assert!(collector.batch(1, vec![]).is_err());
        collector.begin(1).unwrap();
        assert!(collector.begin(1).is_err());
        collector.batch(1, vec![record(1)]).unwrap();
        assert!(collector.batch(1, vec![record(1)]).is_err());

        let mut collector = ReportCollector::default();
        collector.begin(1).unwrap();
        assert!(collector.end(2).is_err());

        let mut collector = ReportCollector::default();
        collector.begin(1).unwrap();
        assert!(collector
            .batch(1, (0..=INDEX_REPORT_BATCH_SIZE).map(record).collect())
            .is_err());
        assert!(collector.end(1).is_err());
    }
}
