//! Scripted WS node: every `NodeOperation` is answered from a table the test
//! seeds, with real-protocol fallbacks (Create journaling + idempotency,
//! 404 for unknown ids, protocol-sized artifact chunks).

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};

use opencoder_core::fleet::{ExecutionRef, RpcReply};
use serde_json::Value;

#[path = "node/service.rs"]
mod service;

#[derive(Default)]
struct Tables {
    /// execution id -> original CreateExecution request (durable journal).
    journal: HashMap<String, Value>,
    /// execution id -> accepted ExecutionIndex json.
    accepted: HashMap<String, Value>,
    /// execution id -> pinned definition carried by the assignment.
    pinned: HashMap<String, Option<Value>>,
    inspects: HashMap<String, RpcReply>,
    /// (execution id, action) -> reply.
    commands: HashMap<(String, String), RpcReply>,
    /// brain action ("snapshot", "layered_context", "pause", ...) -> reply.
    /// Absent actions keep the default 501 so v3/v4 tests opt in explicitly.
    brain: HashMap<String, RpcReply>,
    maintenance: HashMap<String, RpcReply>,
    /// id -> (rows, finished, more).
    events: HashMap<String, (Vec<Value>, bool, bool)>,
    /// (id, step) -> (rows, finished, more) for `DagStepEvents`.
    step_events: HashMap<(String, String), (Vec<Value>, bool, bool)>,
    /// (id, step) -> raw reply overriding the step events table.
    step_events_status: HashMap<(String, String), RpcReply>,
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
    capability_reply: Option<RpcReply>,
    /// command name ("freeze"/"reopen"/"status") -> reply overriding the
    /// default admission behaviour (default side effects are skipped).
    admissions: HashMap<String, RpcReply>,
    /// NodeSnapshot field overrides applied in `snapshot`.
    snapshot_loops: Option<u64>,
    snapshot_ready: Option<bool>,
}

/// One `NodeOperation::Brain` the mock node answered.
#[derive(Clone, Debug)]
pub struct BrainCall {
    pub execution: ExecutionRef,
    pub action: String,
    pub input: Value,
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
    /// Brain actions the node answered (v4 reads/commands included), in order.
    brain_calls: Mutex<Vec<BrainCall>>,
    /// Registration opt-in for `ExecutionKind::Brain`. The kinds list is sent
    /// once with the handshake, so it must be chosen before the node links.
    brain_kind: bool,
    revision: tokio::sync::watch::Sender<u64>,
}

impl MockNode {
    pub fn new(id: &str) -> Arc<Self> {
        Self::build(id, false)
    }

    /// Node that also advertises `ExecutionKind::Brain` in its registration,
    /// so brain runs (v3 or v4) can be placed on it.
    pub fn with_brain_kind(id: &str) -> Arc<Self> {
        Self::build(id, true)
    }

    fn build(id: &str, brain_kind: bool) -> Arc<Self> {
        let (revision, _) = tokio::sync::watch::channel(0);
        Arc::new(Self {
            id: id.into(),
            open: AtomicBool::new(true),
            freezes: AtomicUsize::new(0),
            snapshot_seq: AtomicU64::new(1),
            tables: Mutex::new(Tables::default()),
            seen: Mutex::new(Vec::new()),
            brain_calls: Mutex::new(Vec::new()),
            brain_kind,
            revision,
        })
    }

    pub fn seen_commands(&self) -> Vec<(String, String, Value)> {
        self.seen.lock().unwrap().clone()
    }

    pub fn set_capability_reply(&self, reply: RpcReply) {
        self.tables.lock().unwrap().capability_reply = Some(reply);
    }

    pub fn set_inspect(&self, id: &str, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .inspects
            .insert(id.into(), RpcReply::ok(body));
    }

    /// Raw-reply inspect override for non-200 degradation tests.
    pub fn set_inspect_reply(&self, id: &str, reply: RpcReply) {
        self.tables
            .lock()
            .unwrap()
            .inspects
            .insert(id.into(), reply);
    }

    /// Scripts one `NodeOperation::Brain` action (v4 layered reads/commands).
    pub fn set_brain(&self, action: &str, status: u16, body: Value) {
        self.tables
            .lock()
            .unwrap()
            .brain
            .insert(action.into(), RpcReply { status, body });
    }

    /// Every brain action the node answered, in arrival order.
    pub fn brain_calls(&self) -> Vec<BrainCall> {
        self.brain_calls.lock().unwrap().clone()
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

    /// Seeds one DAG step's event page (`DagStepEvents`), filtered by `after`.
    pub fn set_step_events(&self, id: &str, step: &str, rows: Vec<Value>, finished: bool) {
        let mut t = self.tables.lock().unwrap();
        t.step_events_status.remove(&(id.into(), step.into()));
        t.step_events
            .insert((id.into(), step.into()), (rows, finished, false));
    }

    /// Like `set_step_events` but also scripts the `more` paging flag.
    pub fn set_step_events_more(
        &self,
        id: &str,
        step: &str,
        rows: Vec<Value>,
        finished: bool,
        more: bool,
    ) {
        let mut t = self.tables.lock().unwrap();
        t.step_events_status.remove(&(id.into(), step.into()));
        t.step_events
            .insert((id.into(), step.into()), (rows, finished, more));
    }

    /// Makes `DagStepEvents` reply with this raw RpcReply (error-frame tests).
    pub fn set_step_events_status(&self, id: &str, step: &str, status: u16, body: Value) {
        let mut t = self.tables.lock().unwrap();
        t.step_events.remove(&(id.into(), step.into()));
        t.step_events_status
            .insert((id.into(), step.into()), RpcReply { status, body });
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

    /// The pinned definition the node received for `id` (None when the id
    /// was never assigned; Some(None) for a definition-less assignment).
    pub fn pinned_definition(&self, id: &str) -> Option<Value> {
        self.tables
            .lock()
            .unwrap()
            .pinned
            .get(id)
            .cloned()
            .flatten()
    }

    /// The journalled CreateExecution request for `id` — the wire-level
    /// proof of what the control plane assigned (e.g. dispatch `input`).
    pub fn journal_request(&self, id: &str) -> Option<Value> {
        self.tables.lock().unwrap().journal.get(id).cloned()
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
