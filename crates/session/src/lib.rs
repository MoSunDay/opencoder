pub mod autopilot;
pub mod bash_guard;
pub mod compaction;
pub mod control_cmd;
pub mod dangling_tools;
pub mod event_sink;
pub mod fork;
pub mod mcp;
pub mod mention_resolve;
pub mod plan_handoff;
pub mod plan_phase;
pub mod prompt;
pub mod resume;
pub mod resume_helpers;
pub mod runner;
pub mod skill_context;
pub mod skill_lifecycle;
pub mod skill_resolve;
pub mod streamline;
pub mod subagent_steer_gate;
pub mod tool_guard;
pub mod tools;

pub use control_cmd::{
    apply as apply_control_cmd, clear_seed_text, is_clear_context_handoff, is_clear_context_seed,
    parse as parse_control_cmd, seed_message, split_control_prefix, ControlCmd,
};
pub use event_sink::{run_flusher, spawn_event_flusher, EventSink};
pub use resume::{generate_title, resume, resume_and_replay};
pub use runner::{run, run_once, run_with_images, SessionEvent};
pub use subagent_steer_gate::{SteerReservation, SubagentSteerGate};
pub use tools::question::QuestionHub;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use opencoder_core::{message::now_ms, Agent, AgentKind, Config, Message, Role};
use opencoder_llm::ChatStream;
use opencoder_store::{SessionMeta, Store};
use tokio_util::sync::CancellationToken;

/// Shared, resettable cancellation token used for turn-level interrupts.
/// Wrapping in `Mutex<CancellationToken>` allows resetting after each use
/// (a bare `CancellationToken` is one-shot). The lock is held only briefly
/// (clone-check-fire), never across an `.await`.
pub type SharedCancel = Arc<Mutex<CancellationToken>>;

/// Cancel all registered child subagents. Returns `true` if at least one child
/// was cancelled (i.e. the registry was non-empty). This unblocks the parent's
/// `run_loop` by causing `run_subagent` to return early with `err("cancelled")`,
/// allowing a pending steer to be absorbed at the next turn boundary.
pub fn fire_child_cancels(child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>) -> bool {
    let map = match child_cancels.lock() {
        Ok(m) => m,
        Err(_) => return false,
    };
    if map.is_empty() {
        return false;
    }
    for token in map.values() {
        token.cancel();
    }
    true
}

/// Fire the hard-cancel token for a single child subagent, keyed by its
/// `call_id`. Unlike `fire_child_cancels` (which cancels ALL children), this
/// targets one child so it can be stopped individually — e.g. when a task
/// timeout fires. Returns `true` if the child was found.
pub fn fire_child_cancel(
    child_cancels: &Arc<Mutex<HashMap<String, CancellationToken>>>,
    call_id: &str,
) -> bool {
    let map = match child_cancels.lock() {
        Ok(m) => m,
        Err(_) => return false,
    };
    match map.get(call_id) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

/// Fire the turn-level cancel token for a single child subagent, keyed by its
/// `call_id`. Unlike `fire_child_cancels` (which cascades all hard-cancel
/// tokens), this targets one child's independent turn token. Used when a
/// turn-level interrupt must break a nested subagent's current LLM/tool turn
/// without ending the parent's run loop. Returns `true` if the child was found.
pub fn fire_child_turn_cancel(
    child_turn_cancels: &Arc<Mutex<HashMap<String, SharedCancel>>>,
    call_id: &str,
) -> bool {
    // Clone the token out of the map before locking it, to avoid nested locks.
    let token = {
        let map = match child_turn_cancels.lock() {
            Ok(m) => m,
            Err(_) => return false,
        };
        match map.get(call_id) {
            Some(tc) => tc.clone(),
            None => return false,
        }
    };
    if let Ok(g) = token.lock() {
        g.cancel();
    }
    true
}

/// Fire the turn-level interrupt for the parent session's own `turn_cancel`
/// token. Unlike [`fire_child_turn_cancel`] (which targets one child by
/// `call_id`), the parent owns a single token. Used by interactive front-ends
/// (TUI `>` steer button) to interrupt the parent's current LLM/tool turn
/// WITHOUT ending the `run_loop` -- the next iteration absorbs a pending steer
/// and continues. No-op if the token is already cancelled.
pub fn fire_turn_cancel(token: &SharedCancel) {
    if let Ok(g) = token.lock() {
        g.cancel();
    }
}

pub struct SessionState {
    pub id: String,
    pub messages: Vec<Message>,
    pub agent: Agent,
    pub model: String,
    pub working_dir: PathBuf,
    pub config: Config,
    pub client: Arc<dyn ChatStream>,
    pub last_usage: opencoder_llm::Usage,
    /// Optional durable store. When set, `record` persists each new message.
    pub store: Option<Arc<dyn Store>>,
    /// Active skill instructions. NOT part of the system prompt — a
    /// transient tail reminder (`skill_context::tail_reminder`) surfaces the
    /// skill at LLM-call time; bodies carrying the `body_with_source`
    /// `> Source:` prefix yield the `[active skill]` path reminder (the
    /// model lazily reads the SKILL.md). `None` means no skill is active.
    /// Set from the TUI `$` picker. One-shot lifetime: an activation lives
    /// only for the run that triggered it — `skill_lifecycle` clears it
    /// (memory + store) when that run ends, so later runs start skill-less.
    pub skill_prompt: Arc<Mutex<Option<String>>>,
    /// Names of skills currently activated via `$name` tokens. Used to
    /// unlock latent tools (ssh_pty) in the runner filter. Shares the
    /// one-shot lifetime of `skill_prompt` (cleared together at run end).
    pub active_skill_names: Arc<Mutex<HashSet<String>>>,
    /// Number of messages already persisted to `store` (loaded on resume).
    persisted_count: usize,
    /// Whether the session row has been created in the store.
    session_created: bool,
    ts_origin: bool,
    /// Optional cancellation token. The run loop checks it at each turn
    /// boundary and stops cleanly when cancelled (web interrupt support).
    pub cancel: Option<CancellationToken>,
    /// Turn-level interrupt token. When fired, breaks the current LLM/tool
    /// turn but does NOT end the `run_loop` -- the next iteration absorbs
    /// pending steers and continues. Every session (including the parent) is
    /// constructed with a fresh token so an interactive front-end (TUI `>`
    /// steer) can interrupt the current turn and force steer absorption.
    /// `run_subagent` replaces it with a registered token for children.
    pub turn_cancel: Option<SharedCancel>,
    /// Registry of child subagent turn-cancel tokens, keyed by `call_id`.
    /// Shared (via `Arc`) with the session handle so external code (TUI event
    /// loop, web handler) can fire a specific child's turn interrupt without
    /// going through the worker task.
    pub child_turn_cancels: Arc<Mutex<HashMap<String, SharedCancel>>>,
    /// Runtime admission gates for child steers, keyed by child `call_id`.
    /// The gate makes a natural child turn completion atomic with respect to
    /// an external Enter-style steer admission.
    pub child_steer_gates: Arc<Mutex<HashMap<String, Arc<SubagentSteerGate>>>>,
    /// Admission gate owned by this session when it runs as a subagent.
    pub steer_gate: Option<Arc<SubagentSteerGate>>,
    /// Registry of child subagent hard-cancel tokens, keyed by `call_id`.
    /// Each entry is a `child_token()` derived from the parent's cancel token,
    /// so a parent double-Esc cascades to children, but a parent steer
    /// (TUI `>` or web POST /prompt) can cancel running children without
    /// ending the parent's own `run_loop`.
    pub child_cancels: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Compaction summary text, persisted to the store so resume can
    /// reconstruct the compacted transcript.
    pub summary: Option<String>,
    /// Number of messages in the store that have been summarized (skipped
    /// on resume). `None` means no compaction has occurred.
    pub summary_seq: Option<i64>,
    /// Image URLs preserved across compaction, mirrored from the persisted
    /// `summary_images_json` so the in-memory state stays coherent with the
    /// store. The authoritative copy lives in the store; this field is used
    /// only for in-memory bookkeeping symmetry with `summary`/`summary_seq`.
    pub summary_images: Vec<String>,
    /// Plan→act handoff boundary: number of store messages predating the
    /// handoff (the plan-mode history). On resume these are trimmed and the
    /// handoff plan instruction is re-attached. `None` = no handoff occurred.
    pub handoff_seq: Option<i64>,
    /// Display text of the handoff plan (plan + optional extra). Used to
    /// reconstruct the synthetic plan instruction on resume and to render the
    /// plan card.
    pub handoff_plan: Option<String>,
    /// Requirements submitted in the current plan-mode phase (lifecycle in
    /// [`crate::plan_phase`]). When > 0, plan prompts get a read-only tag.
    pub plan_input_count: usize,
    /// Pre-compaction plan snapshot; see [`crate::plan_phase`].
    pub plan_snapshot: Option<String>,
    /// User-edited task description text, persisted via the /requirement
    /// slash command so it survives session resume.
    pub requirement: Option<String>,
    /// Shared question/answer rendezvous for the `question` tool: an attached
    /// frontend (TUI) resolves; the tool awaits inside the running turn.
    pub question_hub: Arc<QuestionHub>,
}

impl SessionState {
    pub fn new(
        id: impl Into<String>,
        agent: Agent,
        config: Config,
        client: Arc<dyn ChatStream>,
        working_dir: PathBuf,
    ) -> Self {
        let model = config.model_id().to_string();
        SessionState {
            id: id.into(),
            messages: Vec::new(),
            agent,
            model,
            working_dir,
            config,
            client,
            last_usage: opencoder_llm::Usage::default(),
            store: None,
            skill_prompt: Arc::new(Mutex::new(None)),
            active_skill_names: Arc::new(Mutex::new(HashSet::new())),
            persisted_count: 0,
            session_created: false,
            ts_origin: false,
            cancel: None,
            turn_cancel: Some(Arc::new(Mutex::new(CancellationToken::new()))),
            child_turn_cancels: Arc::new(Mutex::new(HashMap::new())),
            child_steer_gates: Arc::new(Mutex::new(HashMap::new())),
            steer_gate: None,
            child_cancels: Arc::new(Mutex::new(HashMap::new())),
            summary: None,
            summary_seq: None,
            summary_images: Vec::new(),
            handoff_seq: None,
            handoff_plan: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
            question_hub: QuestionHub::new(),
        }
    }

    /// Attach a durable store so subsequent `record` calls persist messages.
    pub fn with_store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Mark that the session row already exists in the store (e.g. created
    /// externally before the run loop starts). Prevents `persist()` from
    /// auto-creating a duplicate row with conflicting metadata.
    /// Share an externally owned question hub (tests, alternate frontends).
    pub fn with_question_hub(mut self, hub: Arc<QuestionHub>) -> Self {
        self.question_hub = hub;
        self
    }

    pub fn mark_session_created(mut self) -> Self {
        self.session_created = true;
        self
    }

    /// Mark this session as ts-owned (e.g. launched via `opencode ts`, which
    /// allocates an id without seeding a session row). On the first `persist`
    /// the session row is written with `agent: None` / `model: None`, the
    /// ts-ownership marker that distinguishes it from normal sessions.
    pub fn ts_origin(mut self) -> Self {
        self.ts_origin = true;
        self
    }

    /// Attach a cancellation token so the run loop stops at the next turn boundary.
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Attach a turn-level interrupt token. When fired, the current LLM/tool
    /// turn is interrupted (the `run_loop` continues to absorb pending steers
    /// rather than breaking like a hard cancel). Every session is constructed
    /// with a fresh token already set; this builder lets a caller supply a
    /// specific token it also holds a clone of (e.g. the TUI keeps a handle so
    /// the `>` steer button can fire it).
    pub fn with_turn_cancel(mut self, token: SharedCancel) -> Self {
        self.turn_cancel = Some(token);
        self
    }

    /// Set the active skill instructions (surfaced as a transient tail
    /// reminder by `skill_context::tail_reminder`; `body_with_source`-
    /// prefixed bodies yield the `[active skill]` path reminder).
    pub fn with_skill(self, skill_prompt: String) -> Self {
        *self.skill_prompt.lock().unwrap() = Some(skill_prompt);
        self
    }

    /// Snapshot the active skill instructions (clones the inner String).
    /// One-shot: the value is only meaningful while the run that activated
    /// the skill is still in flight (see `skill_lifecycle`).
    pub fn skill_prompt_cloned(&self) -> Option<String> {
        self.skill_prompt.lock().unwrap().clone()
    }

    /// Snapshot the set of active skill names (cloned).
    pub fn active_skill_names_cloned(&self) -> HashSet<String> {
        self.active_skill_names.lock().unwrap().clone()
    }

    /// Replace the active skill names set. Called when skill tokens are
    /// resolved (TUI) or inferred (resume).
    pub fn set_active_skill_names(&self, names: HashSet<String>) {
        *self.active_skill_names.lock().unwrap() = names;
    }

    /// Update the active skill instructions in place. `None` clears the
    /// skill. One-shot semantics: whatever is set here is cleared at the end
    /// of the run that observes it (`skill_lifecycle::clear_on_run_end`).
    pub fn set_skill(&self, body: Option<String>) {
        *self.skill_prompt.lock().unwrap() = body;
    }

    /// Apply a hot-reloaded config: swap the client, model, and config in
    /// place. The caller builds `new_client` (e.g. from the new base_url/key)
    /// so this module stays decoupled from the concrete `ChatClient`. Used by
    /// the TUI `/model` menu via `UiCmd::ReloadConfig` at the turn boundary.
    pub fn apply_config_reload(&mut self, new_cfg: Config, new_client: Arc<dyn ChatStream>) {
        self.client = new_client;
        self.model = new_cfg.model_id().to_string();
        self.config = new_cfg;
    }

    /// Apply a hot-reloaded config but keep the existing client. Used when
    /// the new endpoint/client cannot be constructed (e.g. missing api_key)
    /// so that at least the `model` and `config` fields stay consistent with
    /// the on-disk config — the live session keeps the old client until the
    /// next successful reload.
    pub fn apply_config_reload_keep_client(&mut self, new_cfg: Config) {
        self.model = new_cfg.model_id().to_string();
        self.config = new_cfg;
    }

    /// Push a message to the in-memory transcript AND persist it if a store is
    /// attached. Best-effort: persistence errors are logged, not fatal, so a
    /// store hiccup never kills an agent run.
    ///
    /// While the plan agent is active, every real assistant text also updates
    /// the phase-bounded `plan_snapshot` (and mirrors it to the store): the
    /// plan→act handoff reads ONLY that snapshot, so it can never fabricate a
    /// "plan" out of an earlier act-phase answer when the plan turn failed or
    /// was cancelled before producing any output (see `plan_handoff::handoff`).
    pub async fn record(&mut self, msg: Message) {
        if let Some(snapshot) = crate::plan_phase::plan_snapshot_update(&self.agent.kind, &msg) {
            self.plan_snapshot = Some(snapshot);
            self.persist_plan_phase().await;
        }
        self.messages.push(msg.clone());
        if let Err(e) = self.persist(&msg).await {
            tracing::warn!(session_id = %self.id, error = %e, "persist message failed");
        }
    }

    async fn persist(&mut self, msg: &Message) -> Result<()> {
        let store = match self.store.clone() {
            Some(s) => s,
            None => return Ok(()),
        };
        if !self.session_created {
            let now = now_ms();
            let meta = SessionMeta {
                id: self.id.clone(),
                title: first_user_text(self.messages.as_slice()),
                agent: if self.ts_origin {
                    None
                } else {
                    Some(self.agent.name.clone())
                },
                model: if self.ts_origin {
                    None
                } else {
                    Some(self.config.model.clone())
                },
                workdir_hash: None,
                created_at: self.messages.first().map(|m| m.created_at).unwrap_or(now),
                updated_at: now,
                summary: self.summary.clone(),
                summary_seq: self.summary_seq,
                summary_images: vec![],
                handoff_seq: self.handoff_seq,
                handoff_plan: self.handoff_plan.clone(),
                skill: self.skill_prompt_cloned(),
                task_type: None,
                requirement: None,
                plan_snapshot: self.plan_snapshot.clone(),
                plan_input_count: self.plan_input_count as i64,
            };
            store.create_session(&meta).await?;
            self.session_created = true;
        }
        store.append_message(&self.id, msg).await?;
        self.persisted_count = self.messages.len();
        Ok(())
    }

    /// Count of messages persisted to the store, accounting for any in-memory-only
    /// synthetic summary at the head. Mirrors the accounting in compaction and
    /// plan_handoff: if a prior compaction set summary_seq, the synthetic
    /// summary message is NOT in the store, so the store count is
    /// summary_seq + (messages.len() - 1).
    pub fn store_message_count(&self) -> usize {
        // The head of the in-memory transcript may be a synthetic message that
        // is NOT in the store: a compaction summary (summary_seq) or a
        // plan->act handoff / clear-context marker (handoff_seq). In both
        // cases the true store count is `skip + len - 1`; with no synthetic
        // head every in-memory message is already persisted.
        if let Some(skip) = self.summary_seq {
            skip as usize + self.messages.len().saturating_sub(1)
        } else if let Some(skip) = self.handoff_seq {
            skip as usize + self.messages.len().saturating_sub(1)
        } else {
            self.messages.len()
        }
    }

    /// Update bookkeeping after compaction. Sets the summary metadata and
    /// adjusts `persisted_count` so subsequent `record()` calls don't try to
    /// re-append already-persisted tail messages.
    pub fn after_compaction(&mut self, summary: String, summary_seq: i64) {
        self.summary = Some(summary);
        self.summary_seq = Some(summary_seq);
        // Compaction subsumes any prior handoff boundary: clear the stale
        // handoff state so resume takes the compaction path, not the handoff
        // path (resume checks handoff_seq first).
        self.handoff_seq = None;
        self.handoff_plan = None;
        self.persisted_count = self.messages.len();
        // The model-reported usage from before the fold measures the old
        // transcript; keep it from re-triggering `should_compact` against
        // the collapsed one (which has nothing left to summarize).
        self.last_usage = opencoder_llm::Usage::default();
    }

    /// Update bookkeeping after a plan→act handoff. Records the handoff
    /// boundary (so resume can trim the plan-mode history) and clears any
    /// compaction state — handoff is the dominant reset, replacing the whole
    /// transcript.
    pub fn after_handoff(&mut self, handoff_seq: i64, handoff_plan: String) {
        self.handoff_seq = Some(handoff_seq);
        self.handoff_plan = Some(handoff_plan);
        self.summary = None;
        self.summary_seq = None;
        self.persisted_count = self.messages.len();
        // The transcript was just collapsed to a fresh start: the reported
        // usage of the discarded history is stale. Leaving it in place made
        // `should_compact` fire against the single-message handoff
        // transcript, which has nothing to summarize, and the runner then
        // killed the run with "compaction failed: transcript exceeds context
        // window but compaction found nothing to summarize".
        self.last_usage = opencoder_llm::Usage::default();
        // Plan phase ended (lifecycle: `crate::plan_phase`).
        self.plan_input_count = 0;
        self.plan_snapshot = None;
    }

    /// When in plan mode and this is not the first requirement in the current
    /// plan phase, append a read-only reminder so the model stays focused on
    /// planning across multi-turn plan conversations. Also increments the
    /// counter so the next call knows this requirement already occurred.
    ///
    /// Recording a new requirement also RETIRES any `plan_snapshot` carried
    /// from an earlier phase (the `ecce7b0` guard): the snapshot survives the
    /// plain act→plan switch, but once a new requirement is submitted the old
    /// plan is stale — if this turn then fails or is cancelled before the
    /// model answers, `handoff` must not hand the old plan forward as if it
    /// answered the new requirement. A successful turn re-captures a fresh
    /// snapshot via `record`. Callers persist the mirror right after this
    /// (`runner::run_with_registry` and `skill_resolve::record_compound`).
    pub fn maybe_tag_plan_prompt(&mut self, text: &mut String) {
        if self.agent.kind == AgentKind::Plan {
            if self.plan_input_count > 0 {
                text.push_str("\n（当前处于只读的 plan 模式，聚焦计划生成）");
            }
            self.plan_input_count += 1;
            self.plan_snapshot = None;
        }
    }
}

fn first_user_text(msgs: &[Message]) -> Option<String> {
    msgs.iter()
        .find(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text().chars().take(80).collect())
}

/// Derive the per-agent prefix-cache salt for `session`, or `None` when the
/// feature is disabled via config. The salt is `<agent_name>:<session_id>` —
/// stable across an agent's turns within a conversation so a prefix-cache
/// backend can keep growing the cached prefix turn over turn. Subagents pass
/// their own child `SessionState` (their `agent.name` is the subagent type and
/// their `id` is `sub-<ULID>`), so each subagent run gets an independent cache
/// namespace.
pub(crate) fn cache_salt_for(session: &SessionState) -> Option<String> {
    (session.config.cache_salt == Some(true))
        .then(|| format!("{}:{}", session.agent.name, session.id))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
