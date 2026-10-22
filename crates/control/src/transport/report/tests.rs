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
    collector.committed(1);
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
    assert!(
        collector
            .batch(1, (0..=INDEX_REPORT_BATCH_SIZE).map(record).collect())
            .is_err()
    );
    assert!(collector.end(1).is_err());
}

#[test]
fn delta_requires_commit_and_tracks_removals_without_pending_recovery() {
    let mut collector = ReportCollector::default();
    assert!(
        collector
            .begin_sync(1, true, IndexReportMode::Delta, Some(1))
            .is_err()
    );
    collector
        .begin_sync(1, true, IndexReportMode::Full, None)
        .unwrap();
    collector.set_pending_at_begin(1, vec![]).unwrap();
    collector.batch(1, vec![record(1), record(2)]).unwrap();
    assert!(collector.end(1).unwrap().unwrap().initial);
    assert!(
        collector
            .begin_sync(2, true, IndexReportMode::Delta, Some(10))
            .is_err(),
        "assembly is not a commit"
    );
    collector.committed(10);
    assert!(
        collector
            .begin_sync(2, false, IndexReportMode::Delta, Some(10))
            .is_err()
    );
    assert!(
        collector
            .begin_sync(2, true, IndexReportMode::Delta, Some(11))
            .is_err()
    );
    collector
        .begin_sync(2, true, IndexReportMode::Delta, Some(10))
        .unwrap();
    collector.batch(2, vec![record(2)]).unwrap();
    collector.removals(2, vec!["agent-1".into()]).unwrap();
    assert!(collector.removals(2, vec!["agent-2".into()]).is_err());
    let delta = collector.end(2).unwrap().unwrap();
    assert!(!delta.initial);
    assert!(delta.pending_at_begin.is_none());
    assert_eq!(delta.base_sequence, Some(10));
    assert_eq!(delta.removed_ids, ["agent-1"]);
    collector.committed(12);
    collector
        .begin_sync(3, true, IndexReportMode::Full, None)
        .unwrap();
    assert!(collector.removals(3, vec!["agent-1".into()]).is_err());
}
