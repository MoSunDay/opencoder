//! Finish an index report without cancelling calls on a superseded Host.
use super::report::CompleteReport;
use crate::AppState;

pub(super) async fn apply(
    state: &AppState,
    id: &str,
    generation: &str,
    sequence: u64,
    complete: &CompleteReport,
) -> anyhow::Result<bool> {
    let mut inventory = complete.inventory.lock().await;
    let fleet = state.fleet.clone();
    let node = id.to_owned();
    let epoch = generation.to_owned();
    let records = complete.records.clone();
    let pending = complete.pending_at_begin.clone();
    let negotiated = complete.negotiated;
    let base_sequence = complete.base_sequence;
    let storage_started = std::time::Instant::now();
    let applied = super::socket_io::storage(async move {
        if negotiated {
            fleet
                .apply_index_report_committed(
                    &node,
                    &records,
                    pending.as_deref(),
                    (&epoch, sequence),
                    base_sequence,
                )
                .await
        } else {
            fleet
                .apply_index_report_fenced(
                    &node,
                    &records,
                    pending.as_deref(),
                    epoch
                        .starts_with("host-")
                        .then_some((epoch.as_str(), sequence)),
                )
                .await
                .map(Some)
        }
    })
    .await;
    let recovered = match applied {
        Ok(recovered) => recovered,
        Err(error)
            if error
                .downcast_ref::<opencoder_store::fleet::IndexReportBaselineChanged>()
                .is_some() =>
        {
            // Another Server committed a different baseline. Reconstruct this
            // peer's complete five-column snapshot before using the shared
            // generation fence; never accept its delta against that baseline.
            let snapshot = inventory.snapshot(complete)?;
            state
                .fleet
                .apply_index_snapshot_fenced(
                    id,
                    &snapshot,
                    complete.pending_at_begin.as_deref(),
                    (generation, sequence),
                )
                .await?
        }
        Err(error) => return Err(error),
    };
    let Some(recovered) = recovered else {
        return Ok(false);
    };
    if negotiated {
        inventory.commit(complete, sequence);
    }
    if storage_started.elapsed().as_millis() >= 250 {
        tracing::info!(node_id = %id, report_id = complete.report_id,
            records = complete.records.len(), removed = complete.removed_ids.len(),
            base_sequence = complete.base_sequence, elapsed_ms = storage_started.elapsed().as_millis(),
            "control index report storage timing");
    }
    // Storage may finish after the socket closes. Only its still-running
    // connection future can promote readiness or acknowledge the handoff.
    // A newer Host can complete its report while this older report awaits
    // storage. Keep the old socket alive for replies to its in-flight calls;
    // its report must not change the new connection's acceptance accounting.
    Ok(state
        .hub
        .commit_report(id, generation, complete, &recovered)
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::SocketCommand;
    use opencoder_core::fleet::*;
    use serde_json::json;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::mpsc;

    fn snapshot(generation: &str) -> NodeSnapshot {
        NodeSnapshot {
            generation: generation.into(),
            sequence: 1,
            cpu_capacity: 1.0,
            active_agent_loops: 0,
            active_runs: 0,
            pending_runs: 0,
            max_runs: 4,
            queue_order: Default::default(),
            ready: true,
            resource_error: None,
        }
    }
    async fn attach(state: &AppState, generation: &str) -> mpsc::Receiver<SocketCommand> {
        let (tx, rx) = mpsc::channel(8);
        state
            .hub
            .attach(
                NodeRegistration {
                    id: "handoff-node".into(),
                    name: "handoff test".into(),
                    version: "test".into(),
                    protocol_version: PROTOCOL_VERSION,
                    maintenance_agent_id: "maintainer".into(),
                    kinds: vec![ExecutionKind::Agent],
                },
                snapshot(generation),
                tx,
            )
            .await
            .unwrap();
        assert!(
            state
                .hub
                .mark_index_synced("handoff-node", generation)
                .await
        );
        rx
    }
    async fn pending(
        state: Arc<AppState>,
        rx: &mut mpsc::Receiver<SocketCommand>,
    ) -> (tokio::task::JoinHandle<RpcReply>, String) {
        let call = tokio::spawn(async move {
            state
                .hub
                .call(
                    "handoff-node",
                    NodeOperation::Maintenance {
                        command: ExecutionCommand {
                            action: "status".into(),
                            input: json!({}),
                        },
                    },
                )
                .await
        });
        let SocketCommand::Frame(frame) = rx.recv().await.unwrap() else {
            panic!("unexpected close")
        };
        let ServerFrame::Call { request_id, .. } = *frame;
        (call, request_id)
    }
    async fn state(root: &std::path::Path) -> Arc<AppState> {
        crate::new_state(root.join("work"), root.join("data"), None)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn superseded_report_keeps_the_old_reply_deliverable() {
        for negotiated in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
            let state = state(root.path()).await;
            let mut old = attach(&state, "host-00000000000000000001-old").await;
            let (call, request) = pending(state.clone(), &mut old).await;
            let _new = attach(&state, "host-00000000000000000002-new").await;
            state
                .fleet
                .apply_index_report_committed(
                    "handoff-node",
                    &[],
                    None,
                    ("host-00000000000000000002-new", 1),
                    None,
                )
                .await
                .unwrap();
            // The old End frame passed its connection check before the new Host
            // completed its report. Finishing that older report is not a failure.
            let applied = apply(
                &state,
                "handoff-node",
                "host-00000000000000000001-old",
                2,
                &CompleteReport {
                    inventory: Default::default(),
                    report_id: 2,
                    records: vec![],
                    pending_at_begin: None,
                    initial: false,
                    negotiated,
                    base_sequence: None,
                    removed_ids: vec![],
                },
            )
            .await;
            assert!(!applied.unwrap());
            state
                .hub
                .resolve(
                    "handoff-node",
                    "host-00000000000000000001-old",
                    &request,
                    RpcReply::ok(json!({"accepted":true})),
                )
                .await;
            let reply = tokio::time::timeout(Duration::from_secs(1), call)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(reply.status, 200);
            assert_eq!(reply.body, json!({"accepted":true}));
            let view = state.hub.views().await.remove(0);
            assert!(view.online && view.snapshot.as_ref().unwrap().ready);
            assert_eq!(
                view.snapshot.unwrap().generation,
                "host-00000000000000000002-new"
            );
        }
    }

    #[tokio::test]
    async fn superseded_disconnect_settles_only_its_own_calls() {
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
        let state = state(root.path()).await;
        let mut old = attach(&state, "host-00000000000000000001-old").await;
        let (old_call, _) = pending(state.clone(), &mut old).await;
        let mut new = attach(&state, "host-00000000000000000002-new").await;
        let (new_call, new_request) = pending(state.clone(), &mut new).await;
        state
            .hub
            .detach("handoff-node", "host-00000000000000000001-old")
            .await;
        let old_reply = tokio::time::timeout(Duration::from_secs(1), old_call)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old_reply.status, 503);
        assert!(!new_call.is_finished());
        state
            .hub
            .resolve(
                "handoff-node",
                "host-00000000000000000002-new",
                &new_request,
                RpcReply::ok(json!({"current":true})),
            )
            .await;
        let reply = new_call.await.unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, json!({"current":true}));
        assert!(state.hub.views().await[0].online);
    }
    #[tokio::test]
    async fn newer_generation_in_shared_storage_does_not_cancel_local_old_calls() {
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
        let state = state(root.path()).await;
        let old_generation = "host-00000000000000000001-old";
        let mut old = attach(&state, old_generation).await;
        let (call, request) = pending(state.clone(), &mut old).await;
        // Another Server has committed the successor, before it attaches here.
        state
            .fleet
            .apply_index_report_committed(
                "handoff-node",
                &[],
                None,
                ("host-00000000000000000002-new", 1),
                None,
            )
            .await
            .unwrap();
        assert!(
            !apply(
                &state,
                "handoff-node",
                old_generation,
                2,
                &CompleteReport {
                    inventory: Default::default(),
                    report_id: 2,
                    records: vec![],
                    pending_at_begin: None,
                    initial: false,
                    negotiated: true,
                    base_sequence: Some(1),
                    removed_ids: vec![],
                }
            )
            .await
            .unwrap()
        );
        state
            .hub
            .resolve(
                "handoff-node",
                old_generation,
                &request,
                RpcReply::ok(json!({"finished":true})),
            )
            .await;
        assert_eq!(call.await.unwrap().status, 200);
    }
}
