//! Mode-aware model resolution and session-only model persistence.

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use opencoder_core::{Agent, AgentKind, AgentMode, Config, ConfigMode};
use opencoder_llm::ChatStream;
use opencoder_store::{SessionModelOverrides, SessionPatch, Store};

use crate::{runner::SessionEvent, SessionState};

/// Return the config view used by a built-in primary Act/Plan agent.
/// Custom agents, subagents, workflows, and auxiliary conversations use Shared.
pub fn config_mode_for_agent(agent: &Agent) -> ConfigMode {
    match (agent.name.as_str(), agent.kind, agent.mode) {
        ("act", AgentKind::Act, AgentMode::Primary) => ConfigMode::Act,
        ("plan", AgentKind::Plan, AgentMode::Primary) => ConfigMode::Plan,
        _ => ConfigMode::Shared,
    }
}

/// Resolve config without mutating the shared base or writing configuration.
pub fn effective_config_for(
    config: &Config,
    agent: &Agent,
    overrides: Option<&SessionModelOverrides>,
) -> Config {
    let mode = config_mode_for_agent(agent);
    let mut effective = match (mode, overrides) {
        // NULL metadata identifies a pre-mode row. Keep every mode on the
        // legacy model instead of retrospectively applying new mode overlays.
        (ConfigMode::Act | ConfigMode::Plan, None) => config.clone(),
        _ => config.effective_for_mode(mode),
    };
    if let Some(model) = overrides.and_then(|items| items.get(mode)) {
        effective.model = model.to_string();
    }
    effective
}

/// Store does not expose a combined model/metadata patch. Update the model in
/// its existing atomic `update_session` call, then metadata; if the second
/// write fails, compensate the first before returning a loud error. Both SQL
/// statements are individually transactional, so a failed metadata statement
/// leaves the old metadata available while the model is restored.
async fn persist_model_row(
    store: &Arc<dyn Store>,
    id: &str,
    model: &str,
    overrides: Option<&SessionModelOverrides>,
) -> Result<()> {
    let previous = store.get_session(id).await?;
    let patch = SessionPatch {
        model: Some(model.to_string()),
        updated_at: Some(opencoder_core::message::now_ms()),
        ..Default::default()
    };
    store.update_session(id, &patch).await?;
    if let Err(error) = store.set_session_model_overrides(id, overrides).await {
        return match store
            .update_session(id, &SessionPatch::rollback_model(previous.as_ref()))
            .await
        {
            Ok(()) => Err(error)
                .context("session model metadata save failed; sessions.model was rolled back"),
            Err(rollback) => Err(anyhow!(
                "session model metadata save failed: {error:#}; sessions.model rollback also failed: {rollback:#}"
            )),
        };
    }
    Ok(())
}

/// Remove residue only after a creation attempted for a row that was confirmed
/// absent. Existing rows take the update path and never reach this cleanup.
async fn compensate_lazy_row(
    store: &Arc<dyn Store>,
    id: &str,
    error: anyhow::Error,
) -> anyhow::Error {
    match store.get_session(id).await {
        Ok(None) => error.context("lazy session row creation failed; no row remained"),
        Ok(Some(_)) => match store.delete_session(id).await {
            Ok(()) => error.context("lazy session row creation failed; new row was rolled back"),
            Err(rollback) => anyhow!(
                "lazy session row creation failed: {error:#}; rollback also failed: {rollback:#}; newly created session row `{id}` remains as residue"
            ),
        },
        Err(inspect) => anyhow!(
            "lazy session row creation failed: {error:#}; could not inspect or roll back a possible new row `{id}`: {inspect:#}; residue may remain"
        ),
    }
}

/// Materialize a previously absent row with its final model/metadata in one
/// insert. Harness persistence is the only follow-up; failure compensates the
/// row creation and reports rollback failure rather than acknowledging memory.
async fn create_lazy_model_row(
    session: &mut SessionState,
    store: &Arc<dyn Store>,
    model: Option<&str>,
    overrides: Option<&SessionModelOverrides>,
) -> Result<()> {
    let mut meta = session.session_meta();
    if let Some(model) = model {
        meta.model = Some(model.to_string());
    }
    match session.write_session_row(store, &meta, overrides).await {
        Ok(()) => {
            session.session_created = true;
            Ok(())
        }
        Err(error) => Err(compensate_lazy_row(store, &session.id, error).await),
    }
}

async fn model_row_exists(session: &SessionState, store: &Arc<dyn Store>) -> Result<bool> {
    if session.session_created {
        return Ok(true);
    }
    Ok(store.get_session(&session.id).await?.is_some())
}

impl SessionState {
    /// Effective config for the current agent, including session-only models.
    pub fn effective_config(&self) -> Config {
        effective_config_for(&self.config, &self.agent, self.model_overrides.as_ref())
    }

    /// Effective configured model (`provider/model` when a provider is present).
    pub fn effective_model(&self) -> String {
        self.effective_config().model
    }

    /// Context window for the active chat; auxiliary model rules are unchanged.
    pub fn effective_context_limit(&self) -> u64 {
        self.effective_config().context_limit()
    }

    /// Current display/model metadata label. It is refreshed before each request.
    pub fn current_model(&self) -> &str {
        &self.model
    }

    /// NULL means legacy single-model metadata; an object means mode-aware.
    pub fn model_overrides(&self) -> Option<&SessionModelOverrides> {
        self.model_overrides.as_ref()
    }

    pub fn model_override(&self, mode: ConfigMode) -> Option<&str> {
        self.model_overrides.as_ref()?.get(mode)
    }

    /// Explicit process/CLI models win in every mode without discarding the
    /// mode-aware generation overlays. Shared/Act/Plan session selections are
    /// cleared, but `Some(default())` remains an empty JSON object; legacy `None` stays
    /// `None`. This synchronous form is store-free and retains lazy-row semantics.
    pub fn apply_explicit_model(&mut self, model: &str) -> bool {
        let previous_model = self.model.clone();
        let previous_active = self.active_model.clone();
        let mut changed = self.config.apply_explicit_model(model);
        if let Some(overrides) = &mut self.model_overrides {
            changed |= overrides.shared.take().is_some();
            changed |= overrides.act.take().is_some();
            changed |= overrides.plan.take().is_some();
        }
        self.reset_effective_model_label();
        self.active_model = self.effective_model();
        changed || self.model != previous_model || self.active_model != previous_active
    }

    /// Apply an explicit model and durably update an already-created row.
    /// Fresh sessions remain lazy: their first normal row creation writes the
    /// in-memory model and empty mode metadata. Any persistence error restores
    /// every in-memory model field so startup cannot continue with a silent
    /// memory-new/disk-old split.
    pub async fn apply_explicit_model_and_persist(&mut self, model: &str) -> Result<bool> {
        let previous_config = self.config.clone();
        let previous_overrides = self.model_overrides.clone();
        let previous_model = self.model.clone();
        let previous_active = self.active_model.clone();
        if !self.apply_explicit_model(model) {
            return Ok(false);
        }
        if self.session_created {
            if let Some(store) = self.store.clone() {
                if let Err(error) =
                    persist_model_row(&store, &self.id, model, self.model_overrides.as_ref()).await
                {
                    self.config = previous_config;
                    self.model_overrides = previous_overrides;
                    self.model = previous_model;
                    self.active_model = previous_active;
                    return Err(error);
                }
            }
        }
        Ok(true)
    }

    /// Copy model inheritance when a frontend creates a related fresh task.
    /// Persistence occurs when that task's session row is first created.
    pub fn inherit_model_overrides(&mut self, overrides: Option<&SessionModelOverrides>) {
        self.model_overrides = overrides.cloned();
        self.reset_effective_model_label();
        self.active_model = self.effective_model();
    }

    /// Copy model inheritance from another in-memory session without writing
    /// either session to the store.
    pub fn inherit_model_overrides_from(&mut self, source: &SessionState) {
        self.inherit_model_overrides(source.model_overrides.as_ref());
    }

    /// Set one session-only Shared/Act/Plan model. Config files are never written.
    pub async fn set_mode_model_override(
        &mut self,
        mode: ConfigMode,
        model: impl Into<String>,
    ) -> Result<()> {
        self.update_mode_model(mode, Some(model.into())).await
    }

    /// Clear one selection, restoring configuration inheritance for that mode.
    pub async fn clear_mode_model_override(&mut self, mode: ConfigMode) -> Result<()> {
        self.update_mode_model(mode, None).await
    }

    async fn update_mode_model(&mut self, mode: ConfigMode, model: Option<String>) -> Result<()> {
        let previous = self.model_overrides.clone();
        let mut next = previous.clone().unwrap_or_else(|| SessionModelOverrides {
            // Converting a legacy row must not change any unselected mode
            // merely because the user edited this one.
            shared: Some(self.config.model.clone()),
            act: (mode != ConfigMode::Act).then(|| self.config.model.clone()),
            plan: (mode != ConfigMode::Plan).then(|| self.config.model.clone()),
        });
        if !next.set(mode, model) {
            bail!("unsupported session model mode");
        }
        self.persist_model_override_change(previous, Some(next))
            .await
    }

    /// Explicit Shared session-only selection, equivalent to the generic mode API.
    pub async fn set_shared_model_override(&mut self, model: impl Into<String>) -> Result<()> {
        self.update_mode_model(ConfigMode::Shared, Some(model.into()))
            .await
    }

    /// Clear only the Shared session selection, retaining Act/Plan selections.
    pub async fn clear_shared_model_override(&mut self) -> Result<()> {
        self.update_mode_model(ConfigMode::Shared, None).await
    }

    /// Update the legacy Shared base in `sessions.model` for full config reloads.
    /// This deliberately does not replace a session-only Shared selection; use
    /// [`Self::set_shared_model_override`] for that operation. A fresh row is
    /// inserted with its final base and metadata together.
    pub async fn set_shared_model(&mut self, model: impl Into<String>) -> Result<()> {
        let model = model.into();
        if let Some(store) = self.store.clone() {
            if model_row_exists(self, &store).await? {
                self.session_created = true;
                let patch = SessionPatch {
                    model: Some(model.clone()),
                    updated_at: Some(opencoder_core::message::now_ms()),
                    ..Default::default()
                };
                store.update_session(&self.id, &patch).await?;
            } else {
                let overrides = self.model_overrides.clone();
                create_lazy_model_row(self, &store, Some(&model), overrides.as_ref()).await?;
            }
        }
        self.config.model = model;
        self.reset_effective_model_label();
        Ok(())
    }

    /// Apply a target environment and persist its Shared base on existing rows,
    /// so a later resume cannot restore the previous Env's `sessions.model`.
    /// Temporary Shared/Act/Plan selections become empty mode-aware metadata. The
    /// two durable fields use the same compensated write path as explicit models.
    /// Fresh rows stay lazy and later materialize both values together.
    pub async fn apply_env_reload(
        &mut self,
        new_config: Config,
        new_client: Option<Arc<dyn ChatStream>>,
    ) -> Result<()> {
        let empty = SessionModelOverrides::default();
        if self.session_created {
            if let Some(store) = self.store.clone() {
                persist_model_row(&store, &self.id, &new_config.model, Some(&empty)).await?;
            }
        }
        self.model_overrides = Some(empty);
        match new_client {
            Some(client) => self.apply_config_reload(new_config, client),
            None => self.apply_config_reload_keep_client(new_config),
        }
        Ok(())
    }

    async fn persist_model_override_change(
        &mut self,
        previous: Option<SessionModelOverrides>,
        next: Option<SessionModelOverrides>,
    ) -> Result<()> {
        if let Some(store) = self.store.clone() {
            if model_row_exists(self, &store).await? {
                // A row discovered behind a lazy handle is pre-existing. Update
                // it in place and, crucially, never use it for compensation.
                self.session_created = true;
                if let Err(error) = store
                    .set_session_model_overrides(&self.id, next.as_ref())
                    .await
                {
                    self.model_overrides = previous;
                    return Err(error);
                }
            } else if let Err(error) =
                create_lazy_model_row(self, &store, None, next.as_ref()).await
            {
                self.model_overrides = previous;
                return Err(error);
            }
        }
        self.model_overrides = next;
        self.reset_effective_model_label();
        Ok(())
    }

    pub(crate) fn reset_effective_model_label(&mut self) {
        self.model = self.effective_config().model_id().to_string();
    }

    /// Consume an effective model change before the next request. Comparing the
    /// full configured model catches provider changes with equal bare model ids;
    /// parameter-only overlays and repeated calls emit nothing.
    pub fn take_model_switch(&mut self) -> Option<SessionEvent> {
        let effective = self.effective_config();
        self.model = effective.model_id().to_string();
        if self.active_model == effective.model {
            return None;
        }
        self.active_model = effective.model;
        Some(SessionEvent::ModelSwitch(self.model.clone()))
    }

    /// Synchronize request identity immediately before LlmRoundStart.
    pub(crate) fn sync_request_model(
        &mut self,
        on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
    ) {
        if let Some(event) = self.take_model_switch() {
            on_event(event);
        }
    }
}
