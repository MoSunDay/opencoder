//! Node index reports: watermark fences, rollback and committed generations.
use crate::report_support::{agent, stored};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_store::fleet::{FleetStore, IndexReportBaselineChanged};

#[tokio::test]
async fn fenced_reports_ignore_stale_watermarks_and_apply_newer_ones() {
    let store = FleetStore::open_memory().await.unwrap();
    let running = agent("agent-a", 1, ExecutionStatus::Running);
    store
        .apply_index_report_fenced("node-a", &[running], None, Some(("host-b", 1)))
        .await
        .unwrap();

    let stale = agent("agent-a", 1, ExecutionStatus::Pending);
    store
        .apply_index_report_fenced(
            "node-a",
            std::slice::from_ref(&stale),
            None,
            Some(("host-b", 1)),
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Running,
        "same generation and sequence is a no-op"
    );
    store
        .apply_index_report_fenced(
            "node-a",
            std::slice::from_ref(&stale),
            None,
            Some(("host-a", 999)),
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Running,
        "an older generation is a no-op at any sequence"
    );

    store
        .apply_index_report(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Idle)],
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Idle
    );
    store
        .apply_index_report_fenced("node-a", &[stale], None, Some(("host-b", 1)))
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Idle,
        "unfenced reports do not advance the watermark"
    );

    store
        .apply_index_report_fenced(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Done)],
            None,
            Some(("host-b", 2)),
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Done
    );
}

#[tokio::test]
async fn failed_fenced_report_rolls_back_rows_and_does_not_advance_the_watermark() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .apply_index_report_fenced(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Idle)],
            None,
            Some(("host-1", 1)),
        )
        .await
        .unwrap();

    let mut conflict = agent("agent-a", 1, ExecutionStatus::Running);
    conflict.kind = ExecutionKind::Team;
    let error = store
        .apply_index_report_fenced(
            "node-a",
            &[agent("agent-new", 2, ExecutionStatus::Idle), conflict],
            None,
            Some(("host-1", 2)),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership conflict"), "{error}");
    assert!(store.index("agent-new").await.unwrap().is_none());
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Idle
    );

    // If the failed report had consumed sequence 2, this report would be stale.
    store
        .apply_index_report_fenced(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Done)],
            None,
            Some(("host-1", 2)),
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Done
    );
}

#[tokio::test]
async fn committed_reports_fence_generations_and_reject_superseded_bases() {
    let store = FleetStore::open_memory().await.unwrap();
    assert!(store
        .apply_index_report_committed(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Pending)],
            None,
            ("gen-1", 1),
            None,
        )
        .await
        .unwrap()
        .is_some());

    store
        .apply_index_report_committed(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Running)],
            None,
            ("gen-1", 2),
            Some(1),
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Running
    );

    let replay = store
        .apply_index_report_committed(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Idle)],
            None,
            ("gen-1", 2),
            Some(1),
        )
        .await
        .unwrap_err();
    assert!(
        replay
            .downcast_ref::<IndexReportBaselineChanged>()
            .is_some(),
        "{replay}"
    );
    assert!(replay.to_string().contains("baseline changed"), "{replay}");
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Running
    );

    let wrong_base = store
        .apply_index_report_committed(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Idle)],
            None,
            ("gen-1", 3),
            Some(1),
        )
        .await
        .unwrap_err();
    assert!(wrong_base
        .downcast_ref::<IndexReportBaselineChanged>()
        .is_some());
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Running
    );

    store
        .apply_index_report_committed(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Done)],
            None,
            ("gen-1", 3),
            Some(2),
        )
        .await
        .unwrap();
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Done
    );

    assert!(
        store
            .apply_index_report_committed(
                "node-a",
                &[agent("agent-a", 1, ExecutionStatus::Error)],
                None,
                ("gen-0", 9),
                Some(2),
            )
            .await
            .unwrap()
            .is_none(),
        "a retired generation is skipped instead of applied"
    );
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Done
    );

    let snapshot = store
        .apply_index_snapshot_fenced(
            "node-a",
            &[agent("agent-a", 1, ExecutionStatus::Error)],
            None,
            ("gen-1", 3),
        )
        .await
        .unwrap();
    assert_eq!(snapshot, Some(Vec::new()), "rebuilt snapshot is a no-op");
    assert_eq!(
        stored(&store, "agent-a").await.status,
        ExecutionStatus::Done
    );
}

#[tokio::test]
async fn committed_report_without_a_durable_base_requires_a_snapshot() {
    let store = FleetStore::open_memory().await.unwrap();
    let error = store
        .apply_index_report_committed("node-a", &[], None, ("gen-1", 1), Some(1))
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<IndexReportBaselineChanged>().is_some());

    store
        .apply_index_report_committed("node-a", &[], None, ("gen-1", 1), None)
        .await
        .unwrap();
    let pending = vec!["agent-pending".to_owned()];
    let error = store
        .apply_index_report_committed("node-a", &[], Some(&pending), ("gen-1", 2), Some(1))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("delta reports cannot recover"),
        "{error}"
    );
}
