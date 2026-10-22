use crate::Worker;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;

#[async_trait::async_trait]
impl NodeService for Worker {
    async fn brain_frames(&self) -> anyhow::Result<Vec<NodeFrame>> {
        crate::brain::outbox::frames(self).await
    }
    fn registration(&self) -> NodeRegistration {
        self.inner.registration.clone()
    }
    fn snapshot(&self) -> NodeSnapshot {
        let error = self
            .inner
            .persistence_error
            .lock()
            .unwrap()
            .clone()
            .or_else(|| {
                self.inner
                    .state
                    .project
                    .require()
                    .ok()
                    .and_then(|deps| deps.persistence_error.lock().unwrap().clone())
            })
            .or_else(|| {
                self.configuration()
                    .and_then(|c| crate::resources::check_mount(c.agent.agents_dir.as_deref()))
                    .err()
                    .map(|e| format!("{e:#}"))
            })
            .or_else(|| self.admission_error());
        NodeSnapshot {
            pending_runs: self
                .inner
                .pending_runs
                .load(std::sync::atomic::Ordering::SeqCst),
            queue_order: self.inner.scheduling.get().queue_order,
            generation: self.inner.generation.clone(),
            sequence: self.next_sequence(),
            cpu_capacity: self.inner.cpu,
            active_agent_loops: opencoder_session::loop_registry::active_ids().len() as u64,
            active_runs: self.active_runs() as u64,
            max_runs: self.inner.scheduling.get().max_runs as u64,
            ready: error.is_none(),
            resource_error: error,
        }
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        opencoder_session::loop_registry::subscribe()
    }
    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        self.cached_indexes().await
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        match crate::operations::handle(self, operation).await {
            Ok(reply) => reply,
            Err(error) => {
                tracing::error!(%error, "node operation failed");
                RpcReply::error(500, format!("{error:#}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkerOptions;
    use opencoder_llm::MockChatClient;
    use opencoder_store::SessionMeta;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[tokio::test]
    async fn node_inventory_reads_more_than_500_sessions_with_activity_cursors() {
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
        let workdir = root.path().join("work");
        std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
        std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
        let worker = Worker::open(
            WorkerOptions {
                name: "inventory".into(),
                workdir,
                data_dir: root.path().join("node"),
                workflow_root: None,
                max_runs: Some(1),
                dag: false,
            },
            Some(Arc::new(MockChatClient::new())),
        )
        .await
        .unwrap();
        for index in 0..503 {
            worker
                .inner
                .state
                .store
                .create_session(&SessionMeta {
                    id: format!("agent-inventory-{index:04}"),
                    created_at: 1000 + index,
                    // Activity sorting differs from creation sorting, including ties.
                    updated_at: if index % 2 == 0 { 2000 } else { 0 },
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        let rows = worker.indexes().await.unwrap();
        let ids: HashSet<_> = rows
            .iter()
            .filter(|r| r.id.starts_with("agent-inventory-"))
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids.len(), 503);
        assert_eq!(
            rows.iter()
                .filter(|r| r.id.starts_with("agent-inventory-"))
                .count(),
            503
        );
        assert!(ids.contains("agent-inventory-0000"));
        assert!(ids.contains("agent-inventory-0502"));
        use opencoder_core::fleet::inventory::InventoryQuery;
        let first = worker
            .inventory_changes(InventoryQuery::default())
            .await
            .unwrap();
        assert!(first.full);
        assert_eq!(first.records.len(), 256);
        worker
            .inner
            .state
            .store
            .delete_session("agent-inventory-0000")
            .await
            .unwrap();
        let tail = worker
            .inventory_changes(InventoryQuery {
                generation: Some(first.generation.clone()),
                after: None,
                page: first.next,
            })
            .await
            .unwrap();
        assert_eq!(tail.records.len(), 247);
        assert_eq!(tail.revision, first.revision);
        let delta = worker
            .inventory_changes(InventoryQuery {
                generation: Some(first.generation.clone()),
                after: Some(first.revision),
                page: None,
            })
            .await
            .unwrap();
        assert!(!delta.full);
        assert!(delta.records.is_empty());
        assert_eq!(delta.removed_ids, vec!["agent-inventory-0000"]);
        let idle = worker
            .inventory_changes(InventoryQuery {
                generation: Some(delta.generation),
                after: Some(delta.revision),
                page: None,
            })
            .await
            .unwrap();
        assert!(idle.records.is_empty() && idle.removed_ids.is_empty());
        assert_eq!(idle.revision, delta.revision);
        worker.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn orphan_dag_root_is_not_reported_as_agent_but_agent_step_is() {
        let root = tempfile::tempdir().unwrap();
        let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
        let workdir = root.path().join("work");
        std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
        std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
        let worker = Worker::open(
            WorkerOptions {
                name: "dag-inventory".into(),
                workdir,
                data_dir: root.path().join("node"),
                workflow_root: None,
                max_runs: Some(1),
                dag: true,
            },
            Some(Arc::new(MockChatClient::new())),
        )
        .await
        .unwrap();
        for (id, title) in [
            ("dag-orphan-root", "harness-native-test"),
            (
                "01KORPHANSTEP00000000000000",
                "dag/dag-orphan-root/diagnose",
            ),
        ] {
            worker
                .inner
                .state
                .store
                .create_session(&SessionMeta {
                    id: id.into(),
                    title: Some(title.into()),
                    kind: Some("dag".into()),
                    created_at: 1000,
                    updated_at: 1000,
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        let indexes = worker.indexes().await.unwrap();
        assert!(!indexes.iter().any(|index| index.id == "dag-orphan-root"));
        assert!(indexes.iter().any(|index| {
            index.id == "01KORPHANSTEP00000000000000" && index.kind == ExecutionKind::Agent
        }));
        worker.shutdown().await.unwrap();
    }
}
