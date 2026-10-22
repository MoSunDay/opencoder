use super::*;
mod performance;

#[test]
fn producer_changes_advance_only_after_commit_and_reconnect_requires_full() {
    let mut state = DeltaState::default();
    assert!(state
        .prepare_source(&report(vec![], 1), 1, false, vec![])
        .is_err());
    let full = report(vec![record(1)], 2);
    assert!(state
        .prepare_source(&full, 10, true, vec![])
        .unwrap()
        .unwrap()
        .base_sequence
        .is_none());
    assert_eq!(state.source_cursor(), None);
    assert!(state.prepare_source(&full, 11, true, vec![]).is_err());
    state.commit();
    assert_eq!(state.source_cursor(), Some(10));
    assert!(state
        .prepare_source(&report(vec![], 3), 10, false, vec![])
        .unwrap()
        .is_none());
    let delta = state
        .prepare_source(
            &report(vec![record(2)], 4),
            11,
            false,
            vec!["agent-1".into()],
        )
        .unwrap()
        .unwrap();
    assert_eq!(delta.base_sequence, Some(2));
    assert_eq!(state.source_cursor(), Some(10));
    state.commit();
    assert_eq!(state.source_cursor(), Some(11));
    state.last_full = Some(Instant::now() - FULL_INTERVAL);
    assert_eq!(state.source_cursor(), None);
    let mut reconnected = DeltaState::default();
    assert!(reconnected
        .prepare_source(&report(vec![record(2)], 5), 11, false, vec![])
        .is_err());
}
fn record(n: usize) -> ExecutionIndex {
    ExecutionIndex {
        id: format!("agent-{n}"),
        created_at: n as i64,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Running,
    }
}

fn report(records: Vec<ExecutionIndex>, sequence: u64) -> PreparedReport {
    let snapshot = NodeSnapshot {
        generation: "generation-a".into(),
        sequence,
        cpu_capacity: 2.0,
        active_agent_loops: 0,
        active_runs: 0,
        pending_runs: 0,
        max_runs: 2,
        queue_order: Default::default(),
        ready: true,
        resource_error: None,
    };
    PreparedReport {
        snapshot,
        records,
        brain: vec![],
    }
}

#[test]
fn twenty_thousand_idle_indexes_send_nothing_and_only_commits_advance_base() {
    let records: Vec<_> = (0..20000).map(record).collect();
    let mut state = DeltaState::default();
    let mut first = report(records.clone(), 5);
    assert!(state
        .prepare(&mut first)
        .unwrap()
        .unwrap()
        .base_sequence
        .is_none());
    assert_eq!(first.records.len(), 20000);
    assert!(
        state.prepare(&mut report(vec![], 6)).is_err(),
        "cannot replace uncommitted baseline"
    );
    state.commit();
    let mut unchanged = report(records.clone(), 7);
    assert!(state.prepare(&mut unchanged).unwrap().is_none());
    assert!(unchanged.records.is_empty());
    let mut changed = records.clone();
    changed[1].status = ExecutionStatus::Done;
    changed.pop();
    let mut next = report(changed.clone(), 9);
    let delta = state.prepare(&mut next).unwrap().unwrap();
    assert_eq!(delta.base_sequence, Some(5));
    assert_eq!(next.records, vec![changed[1].clone()]);
    assert_eq!(delta.removed_ids, vec!["agent-19999"]);
    state.commit();
    state.last_full = Some(Instant::now() - FULL_INTERVAL);
    let mut periodic = report(changed, 10);
    assert!(state
        .prepare(&mut periodic)
        .unwrap()
        .unwrap()
        .base_sequence
        .is_none());
    assert_eq!(periodic.records.len(), 19999);
    // Losing the connection/ACK discards all tentative state.
    let mut reconnected = DeltaState::default();
    assert!(reconnected
        .prepare(&mut report(records, 11))
        .unwrap()
        .unwrap()
        .base_sequence
        .is_none());
}
