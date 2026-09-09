//! Scripted WS node: every `NodeOperation` is answered from a table the test
//! seeds, with real-protocol fallbacks (Create journaling + idempotency,
//! 404 for unknown ids, protocol-sized artifact chunks).

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};

const CHUNK: usize = 64 * 1024;

#[derive(Default)]
struct Tables {
    /// execution id -> original CreateExecution request (durable journal).
    journal: HashMap<String, Value>,
    /// execution id -> accepted ExecutionIndex json.
    accepted: HashMap<String, Value>,
    inspects: HashMap<String, RpcReply>,
    /// (execution id, action) -> reply.
    commands: HashMap<(String, String), RpcReply>,
    maintenance: HashMap<String, RpcReply>,
    /// id -> (rows, finished, more).
    events: HashMap<String, (Vec<Value>, bool, bool)>,
    /// id -> raw reply that overrides the events table (SSE error-frame tests).
    events_status: HashMap<String, RpcReply>,
    /// (id, seq) -> reply (EventPayloadChunk json).
    payloads: HashMap<(String, i64), RpcReply>,
    /// (id, field) -> reply (DetailFieldChunk json).
    fields: HashMap<(String, String), RpcReply>,
    /// id -> reply (MessagePage / items / runs / turns json).
    messages: HashMap<String, RpcReply>,
    todo_items: HashMap<String, RpcReply>,
    project_runs: HashMap<String, RpcReply>,
    team_turns: HashMap<String, RpcReply>,
    /// (id, step, file) -> full bytes; served in protocol-sized chunks.
    artifacts: HashMap<(String, String, String), Vec<u8>>,
    /// (id, step, file) -> per-chunk raw replies; index = offset / 65536.
    /// When present, bypasses protocol-derived chunks entirely.
    artifacts_raw: HashMap<(String, String, String), Vec<RpcReply>>,
    /// When set, Create replies with this raw reply (no journaling).
    create_reply: Option<RpcReply>,
    /// command name ("freeze"/"reopen"/"status") -> reply overriding the
    /// default admission behaviour (default side effects are skipped).
    admissions: HashMap<String, RpcReply>,
    /// NodeSnapshot field overrides applied in `snapshot`.
    snapshot_loops: Option<u64>,
    snapshot_ready: Option<bool>,
}

fn miss404(what: &str) -> RpcReply {
    RpcReply::error(404, what)
}

pub struct MockNode {
    pub id: String,
    pub open: AtomicBool,
    pub freezes: AtomicUsize,
    /// Monotonic snapshot sequence: the hub drops snapshots whose sequence
    /// does not advance, so scripted snapshot overrides must keep growing.
    snapshot_seq: AtomicU64,
    tables: Mutex<Tables>,
    seen: Mutex<Vec<(String, String, Value)>>,
    revision: tokio::sync::watch::Sender<u64>,
}

impl MockNode {
    pub fn new(id: &str) -> Arc<Self> {
        let (revision, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            id: id.into(),
            open: AtomicBool::new(true),
            freezes: AtomicUsize::new(0),
            snapshot_seq: AtomicU64::new(1),
            tables: Mutex::new(Tables::default()),
            seen: Mutex::new(Vec::new()),
            revision,
        })
    }

    pub fn seen_commands(&self) -> Vec<(String, String, Value)> {
        self.seen.lock().unwrap().clone()
    }

    pub fn set_inspect(&self, id: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .inspects
            .insert(id.into(), RpcReply::ok(body));
    }

    pub fn set_command(&self, id: &str, action: &str, status: u16, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .commands
            .insert((id.into(), action.into()), RpcReply { status, body });
    }

    pub fn set_maintenance(&self, action: &str, status: u16, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .maintenance
            .insert(action.into(), RpcReply { status, body });
    }

    pub fn set_events(&self, id: &str, rows: Vec<Value>, finished: bool) {
        let mut t = self.tables.lock().unwrap();
        t.events_status.remove(id);
        t.events.insert(id.into(), (rows, finished, false));
    }

    /// Like `set_events` but also scripts the `more` paging flag in the reply.
    pub fn set_events_more(&self, id: &str, rows: Vec<Value>, finished: bool, more: bool) {
        let mut t = self.tables.lock().unwrap();
        t.events_status.remove(id);
        t.events.insert(id.into(), (rows, finished, more));
    }

    /// Makes the Events operation reply with this raw RpcReply instead of
    /// table rows (clears any seeded rows; `set_events*` clears this back).
    pub fn set_events_status(&self, id: &str, status: u16, body: Value) {
        let mut t = self.tables.lock().unwrap();
        t.events.remove(id);
        t.events_status.insert(id.into(), RpcReply { status, body });
    }

    pub fn set_payload(&self, id: &str, seq: i64, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .payloads
            .insert((id.into(), seq), RpcReply::ok(body));
    }

    pub fn set_field(&self, id: &str, field: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .fields
            .insert((id.into(), field.into()), RpcReply::ok(body));
    }

    pub fn set_messages(&self, id: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .messages
            .insert(id.into(), RpcReply::ok(body));
    }

    pub fn set_todo_items(&self, id: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .todo_items
            .insert(id.into(), RpcReply::ok(body));
    }

    pub fn set_project_runs(&self, id: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .project_runs
            .insert(id.into(), RpcReply::ok(body));
    }

    pub fn set_team_turns(&self, id: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .team_turns
            .insert(id.into(), RpcReply::ok(body));
    }

    pub fn set_artifact(&self, id: &str, step: &str, file: &str, bytes: Vec<u8>) {
        self.tables
            .lock()
            .unwrap()
            .artifacts
            .insert((id.into(), step.into(), file.into()), bytes);
    }

    /// Scripts per-chunk raw replies for one artifact key; chunk index is
    /// `offset / 65536` (clamped to the last reply), bypassing the
    /// protocol-derived chunking of `set_artifact`.
    pub fn set_artifact_raw(&self, id: &str, step: &str, file: &str, replies: Vec<RpcReply>) {
        self.tables
            .lock()
            .unwrap()
            .artifacts_raw
            .insert((id.into(), step.into(), file.into()), replies);
    }

    /// When set, Create replies with this raw RpcReply (no journaling);
    /// `clear_create_reply` restores the normal journal behaviour.
    pub fn set_create_reply(&self, status: u16, body: Value) {
        self.tables.lock().unwrap().create_reply = Some(RpcReply { status, body });
    }

    pub fn clear_create_reply(&self) {
        self.tables.lock().unwrap().create_reply = None;
    }

    /// Journalled execution ids (sorted for deterministic assertions).
    pub fn journal_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .tables
            .lock()
            .unwrap()
            .journal
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// Overrides Freeze/Reopen/Status admission replies by command name
    /// ("freeze"/"reopen"/"status"); when present the default side effects
    /// (open flip, freeze counting) are skipped.
    pub fn set_admission_reply(&self, command: &str, status: u16, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .admissions
            .insert(command.into(), RpcReply { status, body });
    }

    /// Overrides `active_agent_loops`/`ready` in reported NodeSnapshots.
    pub fn set_snapshot_opts(&self, active_agent_loops: Option<u64>, ready: Option<bool>) {
        let mut t = self.tables.lock().unwrap();
        t.snapshot_loops = active_agent_loops;
        t.snapshot_ready = ready;
    }

    pub fn freezes_count(&self) -> usize {
        self.freezes.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl NodeService for MockNode {
    fn registration(&self) -> NodeRegistration {
        NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: self.id.clone(),
            name: self.id.clone(),
            version: "e2e".into(),
            maintenance_agent_id: "act".into(),
            kinds: vec![
                ExecutionKind::Agent,
                ExecutionKind::Dag,
                ExecutionKind::Team,
                ExecutionKind::Todos,
                ExecutionKind::Project,
            ],
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
                    .unwrap_or_else(|| RpcReply::error(400, "unknown execution command"));
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
            NodeOperation::DetailField { request } => t
                .fields
                .get(&(request.execution.id.clone(), request.field.clone()))
                .cloned()
                .unwrap_or_else(|| miss404("detail field not found")),
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
                .unwrap_or_else(|| miss404("project execution not found")),
            NodeOperation::TeamTurns { execution, .. } => t
                .team_turns
                .get(&execution.id)
                .cloned()
                .unwrap_or_else(|| miss404("team execution not found")),
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
