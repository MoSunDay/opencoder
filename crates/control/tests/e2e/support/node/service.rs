use super::{BrainCall, MockNode};
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

const CHUNK: usize = 64 * 1024;

fn miss404(what: &str) -> RpcReply {
    RpcReply::error(404, what)
}

#[async_trait::async_trait]
impl NodeService for MockNode {
    fn registration(&self) -> NodeRegistration {
        let mut kinds = vec![
            ExecutionKind::Agent,
            ExecutionKind::Dag,
            ExecutionKind::Team,
            ExecutionKind::Todos,
            ExecutionKind::Project,
            ExecutionKind::Operator,
        ];
        if self.brain_kind {
            kinds.push(ExecutionKind::Brain);
        }
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: self.id.clone(),
            name: self.id.clone(),
            version: "e2e".into(),
            maintenance_agent_id: "act".into(),
            kinds,
        }
    }

    fn snapshot(&self) -> NodeSnapshot {
        let t = self.tables.lock().unwrap();
        NodeSnapshot {
            pending_runs: 0,
            queue_order: Default::default(),
            generation: format!("{}-g1", self.id),
            sequence: self.snapshot_seq.fetch_add(1, Ordering::SeqCst),
            cpu_capacity: 4.0,
            active_agent_loops: t.snapshot_loops.unwrap_or(0),
            active_runs: 0,
            max_runs: 4,
            ready: t
                .snapshot_ready
                .unwrap_or_else(|| self.open.load(Ordering::SeqCst)),
            resource_error: None,
        }
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision.subscribe()
    }

    async fn indexes(&self) -> anyhow::Result<Vec<ExecutionIndex>> {
        let t = self.tables.lock().unwrap();
        Ok(t.accepted
            .values()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect())
    }

    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        let mut t = self.tables.lock().unwrap();
        match operation {
            NodeOperation::Brain { action, .. } if action == "capability_probe" => {
                t.capability_reply.clone().unwrap_or_else(|| {
                    RpcReply::ok(json!({
                        "compatible": true, "features": ["dag_container_v1", "dag_dynamic_v1", "brain_scheduler_v3"]
                    }))
                })
            }
            NodeOperation::Brain {
                execution,
                action,
                input,
            } => {
                self.brain_calls.lock().unwrap().push(BrainCall {
                    execution,
                    action: action.clone(),
                    input,
                });
                t.brain.get(&action).cloned().unwrap_or_else(|| {
                    RpcReply::error(501, "mock node does not implement brain activations")
                })
            }
            NodeOperation::Create { assignment } => {
                if let Some(reply) = t.create_reply.clone() {
                    return reply;
                }
                let id = assignment.index.id.clone();
                let request = serde_json::to_value(&assignment.request).unwrap();
                if assignment.index.node_id != self.id {
                    return RpcReply::error(409, "assignment ownership or kind mismatch");
                }
                if let Some(prev) = t.journal.get(&id) {
                    return if *prev == request {
                        RpcReply::ok(t.accepted[&id].clone())
                    } else {
                        RpcReply::error(409, "execution id already accepted with different input")
                    };
                }
                let index = serde_json::to_value(&assignment.index).unwrap();
                t.journal.insert(id.clone(), request);
                t.pinned.insert(id.clone(), assignment.definition.clone());
                t.accepted.insert(id, index.clone());
                RpcReply::ok(index)
            }
            NodeOperation::AcceptedRequest { execution } => match t.journal.get(&execution.id) {
                Some(request) => RpcReply::ok(json!({
                    "id": execution.id,
                    "kind": execution.kind,
                    "receipt": request["input"]["brain_receipt"],
                })),
                None => miss404("accepted request not found"),
            },
            NodeOperation::Inspect { execution } => t
                .inspects
                .get(&execution.id)
                .cloned()
                .unwrap_or_else(|| miss404("execution not found")),
            NodeOperation::Command { execution, command } => {
                let action = command.action.clone();
                let input = command.input.clone();
                let reply = t
                    .commands
                    .get(&(execution.id.clone(), action.clone()))
                    .cloned()
                    .unwrap_or_else(|| {
                        if execution.kind == ExecutionKind::Project && action == "project-receipt" {
                            miss404("project run not accepted")
                        } else {
                            RpcReply::error(400, "unknown execution command")
                        }
                    });
                self.seen
                    .lock()
                    .unwrap()
                    .push((execution.id, action, input));
                reply
            }
            NodeOperation::Events { execution, after } => {
                if let Some(reply) = t.events_status.get(&execution.id) {
                    return reply.clone();
                }
                let (rows, finished, more) =
                    t.events
                        .get(&execution.id)
                        .cloned()
                        .unwrap_or((Vec::new(), false, false));
                let filtered: Vec<Value> = rows
                    .into_iter()
                    .filter(|r| r["seq"].as_i64().unwrap_or(0) > after)
                    .collect();
                RpcReply::ok(json!({"events": filtered, "more": more, "finished": finished}))
            }
            NodeOperation::EventPayload { request } => t
                .payloads
                .get(&(request.execution.id.clone(), request.seq))
                .cloned()
                .unwrap_or_else(|| miss404("event payload not found")),
            NodeOperation::DetailField { request } => {
                let key = (request.execution.id.clone(), request.field.clone());
                t.field_reads.push(request.offset);
                if let Some(reply) = t.field_failures.remove(&(request.execution.id.clone(), request.field.clone(), request.offset)) { return reply; }
                if let Some(bytes) = t.field_bytes.get(&key) {
                    use base64::Engine;
                    let offset = request.offset as usize;
                    if offset > bytes.len() { return RpcReply::error(400, "offset out of range"); }
                    let next = bytes.len().min(offset + CHUNK);
                    return RpcReply::ok(json!({"field":request.field,"offset":offset,"next_offset":next,"total_bytes":bytes.len(),"eof":next == bytes.len(),"encoding":"json-base64","bytes_b64":base64::engine::general_purpose::STANDARD.encode(&bytes[offset..next]),"version":opencoder_core::token_hash(std::str::from_utf8(bytes).unwrap())}));
                }
                t.fields.get(&key).cloned().unwrap_or_else(|| miss404("detail field not found"))
            }
            NodeOperation::Messages { execution, .. } => t
                .messages
                .get(&execution.id)
                .cloned()
                .unwrap_or_else(|| miss404("session not found")),
            NodeOperation::TodoItems { execution, .. } => t
                .todo_items
                .get(&execution.id)
                .cloned()
                .unwrap_or_else(|| miss404("workflow not found")),
            NodeOperation::ProjectRuns { execution, .. } => t
                .project_runs
                .get(&execution.id)
                .cloned()
                .unwrap_or_else(|| miss404("execution not found")),
            NodeOperation::TeamTurns { execution, .. } => t
                .team_turns
                .get(&execution.id)
                .cloned()
                .unwrap_or_else(|| miss404("team execution not found")),
            NodeOperation::DagInstances {
                execution,
                step,
                index,
                offset,
                limit,
            } => RpcReply::ok(json!({
                "run_id":execution.id, "step":step, "index":index, "offset":offset, "limit":limit, "instances":[]
            })),
            NodeOperation::DagInstanceEvents {
                execution,
                step,
                index,
                after,
            } => {
                let key = (execution.id, format!("{step}/instances/{index}"));
                if let Some(reply) = t.step_events_status.get(&key) {
                    return reply.clone();
                }
                let (rows, finished, more) =
                    t.step_events
                        .get(&key)
                        .cloned()
                        .unwrap_or((Vec::new(), true, false));
                let rows: Vec<_> = rows
                    .into_iter()
                    .filter(|r| r["seq"].as_i64().unwrap_or(0) > after)
                    .collect();
                RpcReply::ok(json!({"events":rows,"finished":finished,"more":more,"head_seq":0}))
            }
            NodeOperation::DagSteps { execution, step } => t.dag_steps.get(&(execution.id, step.unwrap_or_default())).cloned().unwrap_or_else(|| miss404("dag execution not found")),
            NodeOperation::DagStepEvents {
                execution,
                step,
                after,
            } => {
                let key = (execution.id.clone(), step.clone());
                if let Some(reply) = t.step_events_status.get(&key) {
                    return reply.clone();
                }
                let (rows, finished, more) =
                    t.step_events
                        .get(&key)
                        .cloned()
                        .unwrap_or((Vec::new(), false, false));
                let filtered: Vec<Value> = rows
                    .into_iter()
                    .filter(|r| r["seq"].as_i64().unwrap_or(0) > after)
                    .collect();
                RpcReply::ok(json!({
                    "events": filtered, "more": more, "finished": finished, "head_seq": 0,
                    "step": {"name": step, "status": if finished { "done" } else { "pending" }},
                }))
            }
            NodeOperation::Artifact { request } => {
                let key = (
                    request.execution.id.clone(),
                    request.step.clone(),
                    request.file.clone(),
                );
                if let Some(replies) = t.artifacts_raw.get(&key) {
                    let Some(last_index) = replies.len().checked_sub(1) else {
                        return miss404("artifact not found");
                    };
                    let index = (request.offset as usize / CHUNK).min(last_index);
                    return replies[index].clone();
                }
                let Some(data) = t.artifacts.get(&key) else {
                    return miss404("artifact not found");
                };
                let total = data.len() as u64;
                let offset = request.offset;
                let end = (offset + CHUNK as u64).min(total);
                let slice = &data[offset as usize..end as usize];
                use base64::Engine;
                RpcReply::ok(json!({
                    "step": request.step, "file": request.file,
                    "offset": offset, "next_offset": end, "total_bytes": total,
                    "version": "v1", "eof": end >= total, "encoding": "base64",
                    "bytes_b64": base64::engine::general_purpose::STANDARD.encode(slice),
                }))
            }
            NodeOperation::Admission { command } => {
                let name = match command {
                    NodeAdmissionCommand::Freeze => "freeze",
                    NodeAdmissionCommand::Reopen => "reopen",
                    NodeAdmissionCommand::Status => "status",
                };
                if let Some(reply) = t.admissions.get(name) {
                    return reply.clone();
                }
                match command {
                    NodeAdmissionCommand::Freeze => {
                        self.open.store(false, Ordering::SeqCst);
                        self.freezes.fetch_add(1, Ordering::SeqCst);
                        RpcReply::ok(
                            json!({"mode": "frozen", "active_runs": 0, "owned_processes": 0}),
                        )
                    }
                    NodeAdmissionCommand::Reopen => {
                        self.open.store(true, Ordering::SeqCst);
                        RpcReply::ok(
                            json!({"mode": "open", "active_runs": 0, "owned_processes": 0}),
                        )
                    }
                    NodeAdmissionCommand::Status => RpcReply::ok(json!({
                        "mode": if self.open.load(Ordering::SeqCst) { "open" } else { "frozen" },
                        "active_runs": 0, "owned_processes": 0,
                    })),
                }
            }
            NodeOperation::Maintenance { command } => t
                .maintenance
                .get(&command.action)
                .cloned()
                .unwrap_or_else(|| RpcReply::error(400, "unknown maintenance operation")),
        }
    }
}
