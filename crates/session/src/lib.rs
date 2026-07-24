pub mod bash_guard;
pub mod compaction;
pub mod event_sink;
pub mod plan_handoff;
pub mod prompt;
pub mod resume;
pub mod runner;
pub mod tool_guard;
pub mod tools;

pub use event_sink::{run_flusher, spawn_event_flusher, EventSink};
pub use resume::{generate_title, resume, resume_and_replay};
pub use runner::{run, run_once, run_with_images, SessionEvent};

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use opencoder_core::{message::now_ms, Agent, Config, Message, Role};
use opencoder_llm::ChatStream;
use opencoder_store::{SessionMeta, Store};
use tokio_util::sync::CancellationToken;

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
    /// Names of skills currently activated via `{$name}` tokens. Used to
    /// unlock latent tools (ssh_pty, chrome_headless) in the runner filter.
    pub active_skill_names: Arc<Mutex<HashSet<String>>>,
    /// Number of messages already persisted to `store` (loaded on resume).
    persisted_count: usize,
    /// Whether the session row has been created in the store.
    session_created: bool,
    /// Optional cancellation token. The run loop checks it at each turn
    /// boundary and stops cleanly when cancelled (web interrupt support).
    pub cancel: Option<CancellationToken>,
    /// Compaction summary text, persisted to the store so resume can
    /// reconstruct the compacted transcript.
    pub summary: Option<String>,
    /// Number of messages in the store that have been summarized (skipped
    /// on resume). `None` means no compaction has occurred.
    pub summary_seq: Option<i64>,
    /// Plan→act handoff boundary: number of store messages predating the
    /// handoff (the plan-mode history). On resume these are trimmed and the
    /// handoff plan instruction is re-attached. `None` = no handoff occurred.
    pub handoff_seq: Option<i64>,
    /// Display text of the handoff plan (plan + optional extra). Used to
    /// reconstruct the synthetic plan instruction on resume and to render the
    /// plan card.
    pub handoff_plan: Option<String>,
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
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
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
                handoff_seq: self.handoff_seq,
                handoff_plan: self.handoff_plan.clone(),
                skill: self.skill_prompt_cloned(),
            };
            store.create_session(&meta).await?;
            self.session_created = true;
        }
        store.append_message(&self.id, msg).await?;
        self.persisted_count = self.messages.len();
        Ok(())
    }

    /// Update bookkeeping after compaction. Sets the summary metadata and
    /// adjusts `persisted_count` so subsequent `record()` calls don't try to
    /// re-append already-persisted tail messages.
    pub fn after_compaction(&mut self, summary: String, summary_seq: i64) {
        self.summary = Some(summary);
        self.summary_seq = Some(summary_seq);
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
}
