pub mod bash_guard;
pub mod compaction;
pub mod plan_handoff;
pub mod prompt;
pub mod resume;
pub mod runner;
pub mod tools;

pub use resume::{generate_title, resume, resume_and_replay};
pub use runner::{run, run_once, SessionEvent};

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
            persisted_count: 0,
            session_created: false,
            cancel: None,
            summary: None,
            summary_seq: None,
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
}

fn first_user_text(msgs: &[Message]) -> Option<String> {
    msgs.iter()
        .find(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text().chars().take(80).collect())
}
