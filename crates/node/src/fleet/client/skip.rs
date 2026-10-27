//! Redundant full-report suppression for legacy reporters.
//!
//! Reporters without producer-side commit fences (`supports_index_deltas() ==
//! false`) re-collect their whole inventory every heartbeat. When a fresh
//! collection is identical to the copy the server already acknowledged, the
//! whole update is one snapshot frame: re-uploading unchanged records only
//! burns CPU on both ends. State is connection-local, so a reconnect always
//! starts with a full report.
use super::*;

/// Load-relevant snapshot projection. `sequence` is excluded: every
/// collection bumps it without changing published state.
#[derive(Debug, PartialEq)]
pub(super) struct NodeLoad {
    pub pending_runs: u64,
    pub queue_order: QueueOrder,
    pub generation: String,
    pub cpu_capacity: f64,
    pub active_agent_loops: u64,
    pub active_runs: u64,
    pub max_runs: u64,
    pub ready: bool,
    pub resource_error: Option<String>,
}

impl NodeLoad {
    fn of(snapshot: &NodeSnapshot) -> Self {
        Self {
            pending_runs: snapshot.pending_runs,
            queue_order: snapshot.queue_order,
            generation: snapshot.generation.clone(),
            cpu_capacity: snapshot.cpu_capacity,
            active_agent_loops: snapshot.active_agent_loops,
            active_runs: snapshot.active_runs,
            max_runs: snapshot.max_runs,
            ready: snapshot.ready,
            resource_error: snapshot.resource_error.clone(),
        }
    }
}

/// The inventory a started full report leaves on the server once delivered.
#[derive(Default)]
pub(super) struct PublishedInventory {
    records: Vec<ExecutionIndex>,
    load: Option<NodeLoad>,
}

impl PublishedInventory {
    /// Records the report payload as the server's soon-to-be copy. Reports
    /// either complete or tear down the connection, so capturing at
    /// publication start never leaves a stale baseline behind.
    pub(super) fn capture(&mut self, report: &mut PreparedReport) {
        let records = std::mem::take(&mut report.records);
        self.records = records.clone();
        report.records = records;
        self.load = Some(NodeLoad::of(&report.snapshot));
    }

    /// True when a freshly collected full report carries nothing the server
    /// has not already acknowledged — not even brain frames count, those are
    /// delivered outside index reports.
    pub(super) fn covers(&self, report: &PreparedReport) -> bool {
        self.load.as_ref() == Some(&NodeLoad::of(&report.snapshot)) && self.records == report.records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(index: usize) -> ExecutionIndex {
        ExecutionIndex {
            id: format!("01TESTINDEX{index:011}"),
            created_at: index as i64,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status: ExecutionStatus::Running,
        }
    }

    fn report(records: Vec<ExecutionIndex>, snapshot: NodeSnapshot) -> PreparedReport {
        PreparedReport {
            snapshot,
            records,
            brain: Vec::new(),
        }
    }

    fn snapshot(sequence: u64, ready: bool) -> NodeSnapshot {
        NodeSnapshot {
            pending_runs: 1,
            queue_order: QueueOrder::Fifo,
            generation: "generation-a".into(),
            sequence,
            cpu_capacity: 2.0,
            active_agent_loops: 1,
            active_runs: 1,
            max_runs: 2,
            ready,
            resource_error: None,
        }
    }

    #[test]
    fn unchanged_report_is_covered_regardless_of_sequence() {
        let mut published = PublishedInventory::default();
        let mut first = report(vec![record(1)], snapshot(2, true));
        published.capture(&mut first);
        assert!(published.covers(&report(vec![record(1)], snapshot(9, true))));
    }

    #[test]
    fn changed_records_status_or_empty_baseline_force_publication() {
        let mut published = PublishedInventory::default();
        assert!(!published.covers(&report(vec![], snapshot(1, true))));
        let mut first = report(vec![record(1)], snapshot(2, true));
        published.capture(&mut first);
        assert!(!published.covers(&report(vec![record(2)], snapshot(3, true))));
        assert!(!published.covers(&report(vec![], snapshot(3, true))));
        assert!(!published.covers(&report(vec![record(1)], snapshot(3, false))));
    }

    #[test]
    fn capture_preserves_the_report_payload() {
        let mut published = PublishedInventory::default();
        let mut first = report(vec![record(1), record(2)], snapshot(1, true));
        published.capture(&mut first);
        assert_eq!(first.records.len(), 2);
    }
}
