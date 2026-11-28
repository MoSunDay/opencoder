//! Store-failure contracts for acknowledged model/Env operations and ReloadConfig.

use std::sync::Arc;

use anyhow::Result;
use opencoder_core::{resolve_agent, Config, ConfigMode, Message, ProviderConfig};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{
    Delivery, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
    SessionModelOverrides, SessionPatch, Store, SubagentTaskRecord, TursoStore,
};
use tokio::sync::mpsc;

use super::*;

struct FailingStore {
    inner: Arc<TursoStore>,
    update: bool,
    overrides: bool,
}

#[async_trait::async_trait]
impl Store for FailingStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, value: &SessionMeta) -> Result<()> {
        self.inner.create_session(value).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(filter).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        if self.update {
            anyhow::bail!("injected sessions.model failure")
        }
        self.inner.update_session(id, patch).await
    }
    async fn set_session_model_overrides(
        &self,
        id: &str,
        overrides: Option<&SessionModelOverrides>,
    ) -> Result<()> {
        if self.overrides {
            anyhow::bail!("injected override failure")
        }
        self.inner.set_session_model_overrides(id, overrides).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, keep: &str) -> Result<u64> {
        self.inner.clear_other_sessions(keep).await
    }
    async fn append_message(&self, id: &str, message: &Message) -> Result<i64> {
        self.inner.append_message(id, message).await
    }
    async fn append_messages(&self, id: &str, messages: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(id, messages).await
    }
    async fn load_messages(&self, id: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(id).await
    }
    async fn last_message_seq(&self, id: &str) -> Result<i64> {
        self.inner.last_message_seq(id).await
    }
    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        self.inner.admit_input(input).await
    }
    async fn pending_inputs(&self, id: &str, delivery: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(id, delivery).await
    }
    async fn promote_inputs(&self, id: &str, upto: i64, delivery: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(id, upto, delivery).await
    }
    async fn promote_next_queued(&self, id: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(id).await
    }
    async fn claim_next_queue(&self, id: &str) -> Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(id).await
    }
    async fn delete_input(&self, id: i64) -> Result<()> {
        self.inner.delete_input(id).await
    }
    async fn swap_input_order(&self, id: &str, a: i64, b: i64) -> Result<()> {
        self.inner.swap_input_order(id, a, b).await
    }
    async fn append_events(&self, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(events).await
    }
    async fn events_after(&self, id: &str, seq: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(id, seq).await
    }
    async fn last_event_seq(&self, id: &str) -> Result<i64> {
        self.inner.last_event_seq(id).await
    }
    async fn create_subagent_task(&self, record: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(record).await
    }
    async fn complete_subagent_task(&self, id: &str, result: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(id, result, ok).await
    }
    async fn list_subagent_tasks(&self, id: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(id).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
}

fn config() -> Config {
    Config {
        model: "openai/shared".into(),
        provider: ProviderConfig {
            base_url: "https://example.invalid/v1".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn session(id: &str, update: bool, overrides: bool) -> SessionState {
    let store: Arc<dyn Store> = Arc::new(FailingStore {
        inner: Arc::new(TursoStore::open_memory().await.unwrap()),
        update,
        overrides,
    });
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        config(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store)
}

fn apply(model: &str) -> (UiCmd, mpsc::UnboundedReceiver<Result<(), String>>) {
    let (ack, completion) = mpsc::unbounded_channel();
    (
        UiCmd::ApplySession {
            config: Box::new(config()),
            selection: Some((ConfigMode::Shared, model.into())),
            provider_changed: false,
            ack,
        },
        completion,
    )
}

#[tokio::test]
async fn session_override_store_failure_acknowledges_error_without_worker_mutation() {
    let mut sess = session("session-store-failure", false, true).await;
    sess.set_shared_model("openai/shared").await.unwrap();
    let client = Arc::as_ptr(&sess.client);
    let (events, mut event_rx) = mpsc::channel(8);
    let (command, mut completion) = apply("openai/rejected");

    process_cmd(command, &mut sess, &events, 0).await;

    let error = completion.recv().await.unwrap().unwrap_err();
    assert!(error.contains("injected override failure"), "{error}");
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "openai/shared");
    assert_eq!(Arc::as_ptr(&sess.client), client);
    assert!(
        event_rx.try_recv().is_err(),
        "no global success/error event"
    );
}

#[tokio::test]
async fn reload_config_store_failure_acknowledges_error_without_worker_mutation() {
    let mut sess = session("reload-store-failure", true, false).await;
    sess.set_shared_model("openai/shared").await.unwrap();
    let client = Arc::as_ptr(&sess.client);
    let (events, mut event_rx) = mpsc::channel(8);
    let mut next = config();
    next.model = "openai/rejected".into();

    let (command, mut completion) = acknowledged_config_reload(next, ConfigReloadScope::Full);
    process_cmd(command, &mut sess, &events, 0).await;

    let error = completion.recv().await.unwrap().unwrap_err();
    assert!(error.contains("sessions.model"), "{error}");
    assert!(error.contains("keeping previous runtime config"), "{error}");
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "openai/shared");
    assert_eq!(Arc::as_ptr(&sess.client), client);
    assert!(
        event_rx.try_recv().is_err(),
        "correlated failure must not emit global success/error events"
    );
}

#[tokio::test]
async fn reload_config_client_failure_happens_before_store_or_worker_mutation() {
    let mut sess = session("reload-client-before-store", true, false).await;
    let client = Arc::as_ptr(&sess.client);
    let (events, mut event_rx) = mpsc::channel(8);
    let mut next = config();
    next.model = "openai/rejected".into();
    next.network.proxy = Some("://invalid".into());
    let (command, mut completion) = acknowledged_config_reload(next, ConfigReloadScope::Full);

    process_cmd(command, &mut sess, &events, 0).await;

    let error = completion.recv().await.unwrap().unwrap_err();
    assert!(error.contains("client build failed"), "{error}");
    assert!(!error.contains("sessions.model"), "{error}");
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(Arc::as_ptr(&sess.client), client);
    assert!(event_rx.try_recv().is_err());
}

#[tokio::test]
async fn cli_and_skill_reloads_preserve_session_models_providers_modes_and_client() {
    let mut sess = session("domain-only-reloads", true, false).await;
    sess.config.providers.insert(
        "session-only".into(),
        ProviderConfig {
            base_url: "https://example.invalid/session".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
    );
    sess.config.modes.act = Some(opencoder_core::ModeConfig {
        model: Some("session-only/act".into()),
        ..Default::default()
    });
    let client = Arc::as_ptr(&sess.client);
    let (events, mut event_rx) = mpsc::channel(8);

    let mut cli_candidate = Config {
        model: "openai/disk-must-not-apply".into(),
        provider: ProviderConfig::default(),
        ..Default::default()
    };
    cli_candidate.cli.insert(
        "new-cli".into(),
        opencoder_core::CliConfig {
            enabled: true,
            content: "generic usage".into(),
            ..Default::default()
        },
    );
    let (command, mut completion) =
        acknowledged_config_reload(cli_candidate, ConfigReloadScope::Cli);
    process_cmd(command, &mut sess, &events, 0).await;
    assert_eq!(completion.recv().await, Some(Ok(())));

    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "session-only/act");
    assert!(sess.config.providers.contains_key("session-only"));
    assert!(sess.config.cli["new-cli"].enabled);
    assert_eq!(Arc::as_ptr(&sess.client), client);

    let mut skill_candidate = Config {
        model: "openai/other-disk-model".into(),
        ..Default::default()
    };
    skill_candidate.skills.insert(
        "new-skill".into(),
        opencoder_core::config::SkillConfig { enabled: true },
    );
    let (command, mut completion) =
        acknowledged_config_reload(skill_candidate, ConfigReloadScope::Skills);
    process_cmd(command, &mut sess, &events, 0).await;
    assert_eq!(completion.recv().await, Some(Ok(())));

    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "session-only/act");
    assert!(sess.config.providers.contains_key("session-only"));
    assert!(sess.config.cli.contains_key("new-cli"));
    assert!(sess.config.skills["new-skill"].enabled);
    assert_eq!(Arc::as_ptr(&sess.client), client);
    assert!(
        event_rx.try_recv().is_err(),
        "domain reloads emit no model event"
    );
}

#[tokio::test]
async fn env_override_store_failure_keeps_worker_and_client_unchanged() {
    let mut sess = session("env-store-failure", false, true).await;
    // Create the durable row while update_session still succeeds. Env reload
    // then reaches the injected set_session_model_overrides failure.
    sess.set_shared_model("openai/shared").await.unwrap();
    let client = Arc::as_ptr(&sess.client);
    let (events, mut event_rx) = mpsc::channel(8);
    let (ack, mut completion) = mpsc::unbounded_channel();
    let mut next = config();
    next.model = "openai/env-rejected".into();

    process_cmd(
        UiCmd::ReloadEnv {
            config: Box::new(next),
            ack,
        },
        &mut sess,
        &events,
        0,
    )
    .await;

    let error = completion.recv().await.unwrap().unwrap_err();
    assert!(error.contains("injected override failure"), "{error}");
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "openai/shared");
    assert_eq!(Arc::as_ptr(&sess.client), client);
    assert!(event_rx.try_recv().is_err(), "no ModelSwitch after failure");
}

/// The deferred (mid-turn `/model`) path has no ack: a store refusal is
/// reported through `ModelCommit` only — no `ModelSwitch` (the runtime stays
/// on the previous model), no global error event, worker memory untouched.
#[tokio::test]
async fn deferred_session_override_store_failure_reports_commit_error_without_worker_mutation() {
    let mut sess = session("deferred-store-failure", false, true).await;
    sess.set_shared_model("openai/shared").await.unwrap();
    let client = Arc::as_ptr(&sess.client);
    let (events, mut event_rx) = mpsc::channel(8);
    let selection = Some((ConfigMode::Shared, "openai/rejected".into()));

    process_cmd(
        UiCmd::ApplySessionDeferred {
            config: Box::new(config()),
            selection: selection.clone(),
        },
        &mut sess,
        &events,
        0,
    )
    .await;

    assert!(matches!(
        event_rx.recv().await,
        Some(UiEvent::ModelCommit {
            selection: reported,
            result: Err(error),
        }) if reported == selection && error.contains("injected override failure")
    ));
    assert!(
        event_rx.try_recv().is_err(),
        "a refused switch emits no ModelSwitch"
    );
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "openai/shared");
    assert_eq!(Arc::as_ptr(&sess.client), client);
}
