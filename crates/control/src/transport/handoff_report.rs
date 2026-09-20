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
    let recovered = state
        .fleet
        .apply_index_report_fenced(
            id,
            &complete.records,
            complete.pending_at_begin.as_deref(),
            generation
                .starts_with("host-")
                .then_some((generation, sequence)),
        )
        .await?;
    // A newer Host can complete its report while this older report awaits
    // storage. Keep the old socket alive for replies to its in-flight calls;
    // its report must not change the new connection's acceptance accounting.
    if !state.hub.mark_index_synced(id, generation).await {
        return Ok(false);
    }
    state.hub.acknowledge_report(id, &complete.records).await;
    if complete.initial {
        state.hub.clear_reservations(&complete.records).await;
    }
    state.hub.clear_reservations(&recovered).await;
    Ok(true)
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
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
        let state = state(root.path()).await;
        let mut old = attach(&state, "host-00000000000000000001-old").await;
        let (call, request) = pending(state.clone(), &mut old).await;
        let _new = attach(&state, "host-00000000000000000002-new").await;
        // The old End frame passed its connection check before the new Host
        // completed its report. Finishing that older report is not a failure.
        let applied = apply(
            &state,
            "handoff-node",
            "host-00000000000000000001-old",
            2,
            &CompleteReport {
                report_id: 2,
                records: vec![],
                pending_at_begin: None,
                initial: false,
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
}
