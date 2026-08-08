pub mod autopilot;
pub mod bash_guard;
pub mod compaction;
pub mod control_cmd;
pub mod dangling_tools;
pub mod event_sink;
pub mod fork;
pub mod plan_handoff;
pub mod prompt;
pub mod resume;
pub mod runner;
pub mod skill_resolve;
pub mod streamline;
pub mod tool_guard;
pub mod tools;

pub use control_cmd::{
    apply as apply_control_cmd, is_clear_context_handoff, parse as parse_control_cmd,
    split_control_prefix, ControlCmd,
};
pub use event_sink::{run_flusher, spawn_event_flusher, EventSink};
pub use resume::{generate_title, resume, resume_and_replay};
pub use runner::{run, run_once, run_with_images, SessionEvent};

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
    /// Active skill instructions, injected into the system prompt each turn.
    /// `None` means no skill is active. Set from the TUI `$` picker.
    pub skill_prompt: Arc<Mutex<Option<String>>>,
    /// Names of skills currently activated via `$name` tokens. Used to
    /// unlock latent tools (ssh_pty) in the runner filter.
    pub active_skill_names: Arc<Mutex<HashSet<String>>>,
    /// Number of messages already persisted to `store` (loaded on resume).
    persisted_count: usize,
    /// Whether the session row has been created in the store.
    session_created: bool,
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
    /// Number of user requirements submitted in the current plan-mode phase.
    /// Reset to 0 when switching *to* plan mode (via `/plan` or agent switch)
    /// or after a plan→act handoff. When > 0, subsequent plan prompts get a
    /// read-only reminder appended so the model stays focused on planning.
    pub plan_input_count: usize,
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
            cancel: None,
            turn_cancel: Some(Arc::new(Mutex::new(CancellationToken::new()))),
            child_turn_cancels: Arc::new(Mutex::new(HashMap::new())),
            child_cancels: Arc::new(Mutex::new(HashMap::new())),
            summary: None,
            summary_seq: None,
            summary_images: Vec::new(),
            handoff_seq: None,
            handoff_plan: None,
            plan_input_count: 0,
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
    pub fn mark_session_created(mut self) -> Self {
        self.session_created = true;
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

    /// Set the active skill instructions, injected into the system prompt.
    pub fn with_skill(self, skill_prompt: String) -> Self {
        *self.skill_prompt.lock().unwrap() = Some(skill_prompt);
        self
    }

    /// Snapshot the active skill instructions (clones the inner String).
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

    /// Update the active skill instructions in place. `None` clears the skill.
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
    pub async fn record(&mut self, msg: Message) {
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
                agent: Some(self.agent.name.clone()),
                model: Some(self.config.model.clone()),
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
        self.plan_input_count = 0;
    }

    /// When in plan mode and this is not the first requirement in the current
    /// plan phase, append a read-only reminder so the model stays focused on
    /// planning across multi-turn plan conversations. Also increments the
    /// counter so the next call knows this requirement already occurred.
    pub fn maybe_tag_plan_prompt(&mut self, text: &mut String) {
        if self.agent.kind == AgentKind::Plan {
            if self.plan_input_count > 0 {
                text.push_str("\n（当前处于只读的 plan 模式，聚焦计划生成）");
            }
            self.plan_input_count += 1;
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
mod cache_salt_tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session(cache_salt: Option<bool>) -> SessionState {
        // `cache_salt_for` never touches the filesystem, so a plain temp path
        // (kept alive for the test's duration by the caller) suffices. We use
        // a stable subdir under the OS temp dir rather than a TempDir so the
        // SessionState owns a valid PathBuf without juggling drop lifetimes.
        let working_dir = std::env::temp_dir().join("opencoder-cache-salt-tests");
        SessionState::new(
            "sess-123",
            resolve_agent("act").unwrap(),
            Config {
                cache_salt,
                ..Config::default()
            },
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            working_dir,
        )
    }

    #[test]
    fn derives_salt_when_enabled() {
        let s = make_session(Some(true));
        assert_eq!(cache_salt_for(&s).as_deref(), Some("act:sess-123"));
    }

    #[test]
    fn no_salt_when_disabled_or_unset() {
        let s = make_session(Some(false));
        assert_eq!(cache_salt_for(&s), None);
        let s = make_session(None);
        assert_eq!(cache_salt_for(&s), None);
    }

    /// Build a fresh SharedCancel (turn-level token wrapper) for tests.
    fn new_shared_cancel() -> SharedCancel {
        Arc::new(std::sync::Mutex::new(CancellationToken::new()))
    }

    /// Check whether a SharedCancel token has been fired.
    fn is_shared_cancelled(tc: &SharedCancel) -> bool {
        tc.lock().map(|g| g.is_cancelled()).unwrap_or(false)
    }

    #[test]
    fn fire_child_cancels_returns_false_on_empty_registry() {
        let registry: Arc<Mutex<HashMap<String, CancellationToken>>> =
            Arc::new(Mutex::new(HashMap::new()));
        assert!(!fire_child_cancels(&registry));
    }

    #[test]
    fn fire_child_cancels_cancels_all_registered_tokens() {
        let t1 = CancellationToken::new();
        let t2 = CancellationToken::new();
        let mut map = HashMap::new();
        map.insert("child-1".to_string(), t1.clone());
        map.insert("child-2".to_string(), t2.clone());
        let registry = Arc::new(Mutex::new(map));

        assert!(fire_child_cancels(&registry));
        assert!(t1.is_cancelled());
        assert!(t2.is_cancelled());
    }

    #[test]
    fn fire_child_turn_cancel_returns_false_on_empty_registry() {
        let registry: Arc<Mutex<HashMap<String, SharedCancel>>> =
            Arc::new(Mutex::new(HashMap::new()));
        assert!(!fire_child_turn_cancel(&registry, "child-x"));
    }

    #[test]
    fn fire_child_turn_cancel_returns_false_for_unknown_call_id() {
        let t1 = new_shared_cancel();
        let mut map = HashMap::new();
        map.insert("child-1".to_string(), t1);
        let registry = Arc::new(Mutex::new(map));
        assert!(!fire_child_turn_cancel(&registry, "child-2"));
    }

    #[test]
    fn fire_child_turn_cancel_fires_only_targeted_token() {
        let t1 = new_shared_cancel();
        let t2 = new_shared_cancel();
        let mut map = HashMap::new();
        map.insert("child-1".to_string(), t1.clone());
        map.insert("child-2".to_string(), t2.clone());
        let registry = Arc::new(Mutex::new(map));

        assert!(fire_child_turn_cancel(&registry, "child-1"));
        assert!(is_shared_cancelled(&t1), "targeted token must be cancelled");
        assert!(
            !is_shared_cancelled(&t2),
            "non-targeted token must stay uncancelled"
        );
    }

    #[test]
    fn fire_turn_cancel_fires_supplied_token() {
        let token: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
        assert!(!token.lock().unwrap().is_cancelled());
        fire_turn_cancel(&token);
        assert!(token.lock().unwrap().is_cancelled());
    }
}

#[cfg(test)]
mod plan_tag_tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_plan_session() -> SessionState {
        let config = Config::default();
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "test",
            resolve_agent("plan").unwrap(),
            config,
            client,
            PathBuf::from("."),
        )
    }

    fn make_act_session() -> SessionState {
        let config = Config::default();
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            config,
            client,
            PathBuf::from("."),
        )
    }

    #[test]
    fn plan_first_prompt_not_tagged() {
        let mut s = make_plan_session();
        let mut text = String::from("build a web app");
        s.maybe_tag_plan_prompt(&mut text);
        assert_eq!(text, "build a web app", "first prompt should not be tagged");
        assert_eq!(s.plan_input_count, 1);
    }

    #[test]
    fn plan_second_prompt_tagged() {
        let mut s = make_plan_session();
        s.plan_input_count = 1;
        let mut text = String::from("also add tests");
        s.maybe_tag_plan_prompt(&mut text);
        assert!(text.contains("（当前处于只读的 plan 模式，聚焦计划生成）"));
        assert_eq!(s.plan_input_count, 2);
    }

    #[test]
    fn act_mode_never_tagged() {
        let mut s = make_act_session();
        s.plan_input_count = 5; // even with prior count, act mode should not tag
        let mut text = String::from("do something");
        s.maybe_tag_plan_prompt(&mut text);
        assert_eq!(text, "do something", "act mode should never tag");
    }

    #[test]
    fn switch_to_plan_resets_count() {
        let mut s = make_plan_session();
        s.plan_input_count = 3;
        // simulate ClearContext handoff reset
        s.after_handoff(0, String::new());
        assert_eq!(s.plan_input_count, 0, "after_handoff resets count");
    }
}

#[cfg(test)]
mod compaction_after_handoff_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session() -> SessionState {
        let config = Config::default();
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            config,
            client,
            PathBuf::from("."),
        )
    }

    /// After a plan→act handoff, compaction must clear the stale handoff
    /// boundary and install a compaction summary instead — otherwise resume
    /// would take the handoff path (it checks `handoff_seq` first) and ignore
    /// the freshly written summary.
    #[test]
    fn compaction_after_handoff_clears_handoff_state() {
        let mut s = make_session();
        // Simulate post-handoff state: handoff_seq set, no compaction yet.
        s.after_handoff(10, "the plan".into());
        assert_eq!(s.handoff_seq, Some(10));
        assert!(s.summary_seq.is_none());

        // prev_skip must fall back to handoff_seq when summary_seq is None.
        let prev_skip = s.summary_seq.or(s.handoff_seq).unwrap_or(0);
        assert_eq!(prev_skip, 10, "prev_skip must use handoff_seq");

        // Simulate compaction producing a summary covering the handoff head.
        s.after_compaction("compacted summary".into(), prev_skip);

        assert_eq!(
            s.summary_seq,
            Some(10),
            "summary_seq should hold the (handoff-derived) skip"
        );
        assert!(s.handoff_seq.is_none(), "handoff_seq must be cleared");
        assert!(s.handoff_plan.is_none(), "handoff_plan must be cleared");
        assert_eq!(s.summary.as_deref(), Some("compacted summary"));
    }

    /// With no prior handoff and no prior compaction, prev_skip is 0.
    #[test]
    fn prev_skip_zero_when_no_compaction_or_handoff() {
        let s = make_session();
        let prev_skip = s.summary_seq.or(s.handoff_seq).unwrap_or(0);
        assert_eq!(prev_skip, 0);
    }

    /// When a compaction summary already exists it takes priority over a
    /// (hypothetical leftover) handoff_seq.
    #[test]
    fn summary_seq_takes_priority_over_handoff_seq() {
        let mut s = make_session();
        s.handoff_seq = Some(5);
        s.summary_seq = Some(20);
        let prev_skip = s.summary_seq.or(s.handoff_seq).unwrap_or(0);
        assert_eq!(prev_skip, 20);
        s.after_compaction("s".into(), 20);
        assert!(s.handoff_seq.is_none());
    }
}

#[cfg(test)]
mod store_message_count_tests {
    use super::*;
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    fn make_session() -> SessionState {
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            PathBuf::from("."),
        )
    }

    #[test]
    fn store_message_count_no_synthetic_head() {
        let mut s = make_session();
        s.messages.push(Message::user("u1", "hi"));
        s.messages.push(Message::assistant("a1"));
        assert_eq!(s.store_message_count(), 2);
    }

    #[test]
    fn store_message_count_with_summary_seq() {
        let mut s = make_session();
        s.summary_seq = Some(5);
        // The synthetic summary at index 0 is NOT in the store, so the
        // store count is 5 + (2 - 1) = 6.
        s.messages.push(Message::user("u1", "summary"));
        s.messages.push(Message::assistant("a1"));
        assert_eq!(s.store_message_count(), 6);
    }

    #[test]
    fn store_message_count_with_handoff_seq() {
        let mut s = make_session();
        s.handoff_seq = Some(3);
        s.messages.push(Message::user("u1", "handoff"));
        s.messages.push(Message::assistant("a1"));
        // 3 + (2 - 1) = 4
        assert_eq!(s.store_message_count(), 4);
    }

    #[test]
    fn store_message_count_empty_with_summary_seq_does_not_overflow() {
        // This is the bug: messages.len() == 0 with summary_seq set would
        // underflow. saturating_sub prevents the panic.
        let mut s = make_session();
        s.summary_seq = Some(5);
        s.messages.clear();
        // skip=5 + saturating_sub(0,1)=0 = 5
        assert_eq!(s.store_message_count(), 5);
    }

    #[test]
    fn store_message_count_empty_with_handoff_seq_does_not_overflow() {
        let mut s = make_session();
        s.handoff_seq = Some(3);
        s.messages.clear();
        assert_eq!(s.store_message_count(), 3);
    }
}
