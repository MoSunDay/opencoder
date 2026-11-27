//! Acknowledged model/Provider, scoped config, and local-Env worker operations.

use std::sync::Arc;

use opencoder_core::{Config, ConfigMode};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::{SessionEvent, SessionState};
use tokio::sync::mpsc;

use super::{forward_event, persist_event, ConfigReloadScope, UiEvent};

fn build_client(config: &Config, operation: &str) -> Result<Arc<dyn ChatStream>, String> {
    let endpoint = config
        .resolve_endpoint()
        .map_err(|error| format!("{operation} endpoint resolve failed: {error:#}"))?;
    let client = ChatClient::from_config(config, &endpoint)
        .map_err(|error| format!("{operation} client build failed: {error:#}"))?;
    Ok(Arc::new(client))
}

async fn persist_selection(
    sess: &mut SessionState,
    mode: ConfigMode,
    model: String,
) -> Result<(), String> {
    let result = match mode {
        ConfigMode::Shared => sess.set_shared_model_override(model).await,
        ConfigMode::Act | ConfigMode::Plan => sess.set_mode_model_override(mode, model).await,
    };
    result.map_err(|error| format!("session model update failed: {error:#}"))
}

/// Emit exactly one effective-model notification after all fallible work has
/// succeeded. Pure model selection uses SessionState's generation ledger;
/// apply_config_reload resets that ledger, so a manual event is needed there.
async fn emit_model_change(
    sess: &mut SessionState,
    ui_tx: &mpsc::UnboundedSender<UiEvent>,
    previous: String,
) {
    if sess.effective_model() == previous {
        return;
    }
    let event = sess
        .take_model_switch()
        .unwrap_or_else(|| SessionEvent::ModelSwitch(sess.effective_config().model_id().into()));
    persist_event(&sess.store, &sess.id, &event).await;
    forward_event(ui_tx, event);
}

pub(super) async fn apply_session(
    sess: &mut SessionState,
    ui_tx: &mpsc::UnboundedSender<UiEvent>,
    config: Config,
    selection: Option<(ConfigMode, String)>,
    provider_changed: bool,
) -> Result<(), String> {
    let previous = sess.effective_model();
    // The worker's SessionState is authoritative. A Provider-only patch must
    // never reinstall a stale outer Shared/model snapshot while rebuilding.
    // Resolve the endpoint from the effective session/selection model, but
    // commit only Provider data to the raw base below.
    let mut provider_config = sess.effective_config();
    provider_config.providers = config.providers.clone();
    if let Some((_, model)) = &selection {
        provider_config.model = model.clone();
    }
    // Construction is deliberately before Store or SessionState mutation.
    // Model-only changes never enter this branch and retain the client pointer.
    let client = provider_changed
        .then(|| build_client(&provider_config, "Provider reload"))
        .transpose()?;
    if let Some((mode, model)) = selection {
        persist_selection(sess, mode, model).await?;
    }
    if let Some(client) = client {
        let mut next = sess.config.clone();
        next.providers = config.providers;
        sess.apply_config_reload(next, client);
    }
    emit_model_change(sess, ui_tx, previous).await;
    Ok(())
}

pub(super) async fn reload_config(
    sess: &mut SessionState,
    ui_tx: &mpsc::UnboundedSender<UiEvent>,
    candidate: Config,
    scope: ConfigReloadScope,
) -> Result<(), String> {
    if scope != ConfigReloadScope::Full {
        // Never trust non-domain fields from a domain-save candidate. In
        // particular, disk Shared values must not reach sessions.model.
        let mut next = sess.config.clone();
        match scope {
            ConfigReloadScope::Cli => next.cli = candidate.cli,
            ConfigReloadScope::Skills => next.skills = candidate.skills,
            ConfigReloadScope::Full => unreachable!("handled below"),
        }
        sess.apply_config_reload_keep_client(next);
        return Ok(());
    }

    let previous = sess.effective_model();
    // Build first: endpoint/client failure must not create or update a session
    // row and must leave both worker config and client untouched.
    let client = build_client(&candidate, "Config reload")?;
    if candidate.model != sess.config.model {
        sess.set_shared_model(candidate.model.clone())
            .await
            .map_err(|error| {
                format!(
                    "config reload failed to update sessions.model: {error:#}; keeping previous runtime config"
                )
            })?;
    }
    sess.apply_config_reload(candidate, client);
    emit_model_change(sess, ui_tx, previous).await;
    sync_mcp(sess).await;
    Ok(())
}

async fn sync_mcp(sess: &SessionState) {
    let desired: Vec<_> = sess
        .config
        .enabled_mcp_servers()
        .into_iter()
        .map(|(name, config)| (name, config.clone()))
        .collect();
    opencoder_session::mcp::pool::sync(&sess.id, &desired).await;
}

pub(super) async fn reload_env(
    sess: &mut SessionState,
    ui_tx: &mpsc::UnboundedSender<UiEvent>,
    config: Config,
) -> Result<(), String> {
    let previous = sess.effective_model();
    let client = build_client(&config, "Env reload")?;
    // SessionState persists the override clear before applying the candidate;
    // on Store failure both worker memory and durable overrides stay unchanged.
    sess.apply_env_reload(config, Some(client))
        .await
        .map_err(|error| format!("Env reload failed: {error:#}"))?;

    sync_mcp(sess).await;
    emit_model_change(sess, ui_tx, previous).await;
    Ok(())
}
