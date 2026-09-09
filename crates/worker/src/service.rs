use crate::Worker;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use opencoder_store::SessionFilter;
use std::collections::HashSet;

#[async_trait::async_trait]
impl NodeService for Worker {
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
                .list_sessions(&SessionFilter {
                    limit: 500,
                    cursor,
                    workdir_hash: None,
                    search: None,
                    include_subagents: true,
                })
                .await?;
            if rows.is_empty() {
                break;
            }
            cursor = rows.last().map(|r| r.id.clone());
            for row in &rows {
                if roots.contains(&row.id) {
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
