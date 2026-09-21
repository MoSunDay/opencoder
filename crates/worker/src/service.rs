use crate::Worker;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use opencoder_store::SessionFilter;
use std::collections::HashSet;

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
        let mut records: Vec<_> = self
            .inner
            .journal
            .lock()
            .await
            .records
            .values()
            .map(|r| r.assignment.index.clone())
            .collect();
        let projects = self.inner.state.project.require()?;
        let journal = self.inner.journal.lock().await;
        for record in journal
            .records
            .values()
            .filter(|r| r.assignment.request.kind == ExecutionKind::Project)
        {
            if let Some(todo) = &record.assignment.request.target {
                for run in projects.projects.list_todo_runs(todo).await? {
                    records.push(ExecutionIndex {
                        id: run.id,
                        created_at: run.created_at,
                        kind: ExecutionKind::Project,
                        node_id: self.inner.registration.id.clone(),
                        status: match run.status {
                            opencoder_store::ProjectTodoRunStatus::Running => {
                                if record.assignment.index.status == ExecutionStatus::Pending {
                                    ExecutionStatus::Pending
                                } else if self
                                    .inner
                                    .active
                                    .lock()
                                    .await
                                    .contains_key(&record.assignment.index.id)
                                {
                                    ExecutionStatus::Running
                                } else {
                                    ExecutionStatus::Interrupted
                                }
                            }
                            opencoder_store::ProjectTodoRunStatus::Done => ExecutionStatus::Done,
                            opencoder_store::ProjectTodoRunStatus::Cancelled => {
                                ExecutionStatus::Cancelled
                            }
                            _ => ExecutionStatus::Error,
                        },
                    });
                }
            }
        }
        drop(journal);
        let roots: HashSet<_> = records.iter().map(|r| r.id.clone()).collect();
        let active: HashSet<_> = opencoder_session::loop_registry::active_ids()
            .into_iter()
            .collect();
        let mut cursor = None;
        loop {
            let rows = self
                .inner
                .state
                .store
                .list_execution_sessions(&SessionFilter {
                    limit: 500,
                    cursor,
                    workdir_hash: None,
                    search: None,
                    // The execution inventory includes DAG Agent-step rows;
                    // the public chat API applies its own visibility fence.
                    // Operator sessions are lane-fenced at the store layer
                    // (their executions live in the node journal).
                    include_subagents: false,
                    kind: None,
                })
                .await?;
            if rows.is_empty() {
                break;
            }
            cursor = rows
                .last()
                .map(|row| format!("{}|{}", row.updated_at.max(row.created_at), row.id));
            for row in &rows {
                if roots.contains(&row.id) {
                    continue;
                }
                // A session that reaches this fallback path has no durable
                // execution journal. Public Agent sessions use `agent-`;
                // Team's node-local member sessions use `member-` and carry
                // the coordinator/member title. Keep both as Agent indexes
                // so their execution can be inspected through the parent
                // Team without exposing them in the public chat lane.
                // Tagged rows resolve at the store layer (the store lane
                // fence already dropped `operator`); untagged legacy rows
                // keep the id-prefix/title resolution below.
                let tagged = row.kind.as_deref();
                if let Some(kind) = tagged {
                    match kind {
                        "agent" | "team" | "dag" => {
                            records.push(ExecutionIndex {
                                id: row.id.clone(),
                                kind: ExecutionKind::Agent,
                                node_id: self.inner.registration.id.clone(),
                                created_at: row.created_at,
                                status: internal_session_status(active.contains(&row.id)),
                            });
                        }
                        _ => {}
                    }
                    continue;
                }
                let member_session = row.id.starts_with("member-")
                    && row
                        .title
                        .as_deref()
                        .is_some_and(|title| title.contains(" / "));
                let dag_step_session = row
                    .title
                    .as_deref()
                    .is_some_and(|title| title.starts_with("dag/"));
                if !row.id.starts_with("agent-") && !member_session && !dag_step_session {
                    continue;
                }
                records.push(ExecutionIndex {
                    id: row.id.clone(),
                    kind: ExecutionKind::Agent,
                    node_id: self.inner.registration.id.clone(),
                    created_at: row.created_at,
                    status: internal_session_status(active.contains(&row.id)),
                });
            }
            if rows.len() < 500 {
                break;
            }
        }
        Ok(records)
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

fn internal_session_status(active: bool) -> ExecutionStatus {
    if active {
        ExecutionStatus::Running
    } else {
        // Sessions without a top-level journal are owned by another
        // execution and cannot accept work directly once their loop exits.
        ExecutionStatus::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkerOptions;
    use opencoder_llm::MockChatClient;
    use opencoder_store::SessionMeta;
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
        worker.shutdown().await.unwrap();
    }
}
