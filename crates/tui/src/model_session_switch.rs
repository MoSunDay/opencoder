//! Session-only model/provider selection without configuration writes.

use std::sync::Arc;

use opencoder_core::{Config, ConfigMode};
use opencoder_llm::ChatStream;
use opencoder_store::SessionModelOverrides;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::theme;
use crate::worker::UiCmd;

fn display_overrides(chat: &ChatView, config: &Config, mode: ConfigMode) -> SessionModelOverrides {
    chat.model_overrides
        .clone()
        .unwrap_or_else(|| SessionModelOverrides {
            // Mirror SessionState::update_mode_model: converting a legacy row
            // pins the base (Shared slot) to the legacy model, while an
            // unselected Act/Plan mode retains that same base rather than a
            // config mode overlay. `/model` never writes the Shared slot
            // itself; this is only the display copy of the session's own
            // legacy conversion.
            shared: Some(config.model.clone()),
            act: (mode != ConfigMode::Act).then(|| config.model.clone()),
            plan: (mode != ConfigMode::Plan).then(|| config.model.clone()),
        })
}

/// Commit one session-only selection into the display metadata. Shared by the
/// idle (`ApplySession` ack) and mid-turn (`ModelOverrideApplied`) paths so
/// both end with the exact same `chat.model` / `shared_model` / menu
/// checkmarks.
fn commit_display(chat: &mut ChatView, config: &Config, mode: ConfigMode, model: &str) {
    let mut overrides = display_overrides(chat, config, mode);
    let _ = overrides.set(mode, Some(model.to_string()));
    chat.model_overrides = Some(overrides);
    chat.shared_model = crate::model_scope::shared_model(config, chat.model_overrides.as_ref());
    chat.model = crate::model_scope::effective_for_chat(config, chat).model;
}

/// Queue a session-only model selection while a turn runs. The pick goes
/// straight onto the session's `ModelSwitchQueue` (never the serial worker
/// command channel, which is only drained between turns): the runner applies
/// it when the current LLM round completes and announces it through
/// `SessionEvent::ModelOverrideApplied`, which the UI fold turns into
/// [`commit_session_switch`]. Nothing is committed here, so the status bar
/// keeps showing the model the in-flight request uses. Provider edits are
/// refused (they rebuild the client and belong to the idle `/model` path).
pub(crate) fn queue_session_switch(
    json: serde_json::Value,
    queue: &opencoder_session::ModelSwitchQueue,
    chat: &mut ChatView,
) {
    if json.get("providers").is_some() {
        marker(
            chat,
            "[/model] busy \u{2014} Provider changes apply when idle; model selection is unchanged"
                .into(),
            true,
        );
        return;
    }
    let Some((mode, model)) = crate::model_scope::selection_from_patch(&json) else {
        marker(
            chat,
            "[/model] session-only change contained no model selection".into(),
            true,
        );
        return;
    };
    opencoder_session::queue_model_switch(queue, mode, model.clone());
    marker(
        chat,
        format!(
            "[/model] {} queued \u{2192} {} (applies when the current LLM round ends)",
            crate::model_scope::mode_name(mode),
            model
        ),
        false,
    );
}

/// Fold the runner's pick notification: `mode` is the `act`/`plan` wire key,
/// `error` a refused pick. `/model` has no Shared scope, so a `shared` (or
/// unknown) key is reported and never applied.
pub(crate) fn commit_model_override(
    chat: &mut ChatView,
    config: &Config,
    mode: &str,
    model: &str,
    error: Option<String>,
) {
    let Some(mode) =
        opencoder_session::mode_from_key(mode).filter(|mode| *mode != ConfigMode::Shared)
    else {
        marker(
            chat,
            format!("[/model] ignored a model pick outside Act/Plan (`{mode}`)"),
            true,
        );
        return;
    };
    commit_session_switch(chat, config, mode, model, error.map_or(Ok(()), Err));
}

/// Commit one applied session-only selection into the display metadata: the
/// same end state as the idle `ApplySession` ack path (`chat.model_overrides`
/// / `shared_model` / menu checkmarks), so the status bar shows exactly the
/// model the next request uses. A refused pick keeps every field unchanged.
pub(crate) fn commit_session_switch(
    chat: &mut ChatView,
    config: &Config,
    mode: ConfigMode,
    model: &str,
    result: Result<(), String>,
) {
    match result {
        Ok(()) => {
            commit_display(chat, config, mode, model);
            marker(
                chat,
                format!(
                    "[/model] {} switched (session only) \u{2192} {}",
                    crate::model_scope::mode_name(mode),
                    model
                ),
                false,
            );
        }
        Err(error) => {
            marker(
                chat,
                format!("[/model] session-only change failed: {error}; runtime unchanged"),
                true,
            );
        }
    }
}

/// Apply only the providers map. `Ok(None)` means the patch has no provider
/// data; `Ok(Some(changed))` controls whether the worker needs ReloadConfig.
fn apply_provider_patch(
    config: &mut Config,
    json: &serde_json::Value,
) -> Result<Option<bool>, String> {
    use serde_json::Value;

    let Some(patch) = json.get("providers") else {
        return Ok(None);
    };
    if patch.is_null() {
        let changed = !config.providers.is_empty();
        config.providers.clear();
        return Ok(Some(changed));
    }
    let entries = patch
        .as_object()
        .ok_or_else(|| "providers must be an object or null".to_string())?;
    let mut base = config.clone();
    for (name, value) in entries {
        match value {
            Value::Null => {
                base.providers.remove(name);
            }
            Value::Object(fields) => {
                if let Some(existing) = base.providers.get_mut(name) {
                    // Config::merged_with appends headers and ignores null API
                    // keys; disk merges replace/delete them first. Mirror that.
                    if fields.contains_key("headers") {
                        existing.headers.clear();
                    }
                    if fields.get("api_key").is_some_and(Value::is_null) {
                        existing.api_key = None;
                    }
                }
            }
            _ => return Err(format!("providers.{name} must be an object or null")),
        }
    }
    let merged = base.merged_with(&serde_json::json!({ "providers": entries }));
    let changed = merged.providers != config.providers;
    config.providers = merged.providers;
    Ok(Some(changed))
}

fn marker(chat: &mut ChatView, text: String, error: bool) {
    let color = if error {
        theme::err_color()
    } else {
        theme::accent()
    };
    chat.push_marker(Line::from(Span::styled(text, Style::default().fg(color))));
}

pub(crate) fn build_session_client(config: &Config) -> Result<Arc<dyn ChatStream>, String> {
    let endpoint = config
        .resolve_endpoint()
        .map_err(|error| format!("endpoint resolve failed: {error:#}"))?;
    let client = opencoder_llm::ChatClient::from_config(config, &endpoint)
        .map_err(|error| format!("client build failed: {error:#}"))?;
    Ok(Arc::new(client))
}

pub(crate) async fn switch_session(
    json: serde_json::Value,
    config: &mut Config,
    client: &mut Arc<dyn ChatStream>,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
) {
    // Keep the outer base identical to SessionState::config. Session-only
    // model selections live in ChatView metadata; only Provider data changes
    // the shared base. Client construction separately uses the effective view.
    let mut next = config.clone();
    let providers = match apply_provider_patch(&mut next, &json) {
        Ok(changed) => changed,
        Err(error) => {
            marker(
                chat,
                format!("[/model] invalid session-only Provider change: {error}"),
                true,
            );
            return;
        }
    };
    let selection = crate::model_scope::selection_from_patch(&json);
    if providers.is_none() && selection.is_none() {
        marker(
            chat,
            "[/model] session-only change contained neither Provider data nor a model selection"
                .into(),
            true,
        );
        return;
    }
    // Prebuild the outer client without committing it. Worker construction can
    // still fail independently; in that case both candidates are dropped.
    let next_client = if providers == Some(true) {
        let mut client_config = crate::model_scope::effective_for_chat(&next, chat);
        if let Some((_, model)) = &selection {
            client_config.model = model.clone();
        }
        match build_session_client(&client_config) {
            Ok(client) => Some(client),
            Err(error) => {
                marker(
                    chat,
                    format!("[/model] session-only Provider reload failed: {error}"),
                    true,
                );
                return;
            }
        }
    } else {
        None
    };
    let (ack, mut completion) = mpsc::unbounded_channel();
    let command = UiCmd::ApplySession {
        config: Box::new(next.clone()),
        selection: selection.clone(),
        provider_changed: providers == Some(true),
        ack,
    };
    if cmd_tx.send(command).await.is_err() {
        marker(
            chat,
            "[/model] session-only change failed: worker command channel closed; runtime unchanged"
                .into(),
            true,
        );
        return;
    }
    match completion.recv().await {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            marker(
                chat,
                format!("[/model] session-only change failed: {error}; runtime unchanged"),
                true,
            );
            return;
        }
        None => {
            marker(
                chat,
                "[/model] session-only change failed: worker ended without confirmation; runtime unchanged"
                    .into(),
                true,
            );
            return;
        }
    }

    if let Some(new_client) = next_client {
        *client = new_client;
    }
    *config = next;
    if let Some((mode, model)) = &selection {
        commit_display(chat, config, *mode, model);
    }

    if let Some((mode, model)) = selection {
        let text = if providers.is_some() {
            format!(
                "[/model] Provider + {} selection applied (session only) → {}",
                crate::model_scope::mode_name(mode),
                model
            )
        } else {
            format!(
                "[/model] {} switched (session only) → {}",
                crate::model_scope::mode_name(mode),
                model
            )
        };
        marker(chat, text, false);
    } else {
        marker(
            chat,
            "[/model] Provider configuration applied (session only)".into(),
            false,
        );
    }
}

#[cfg(test)]
#[path = "model_session_switch_tests.rs"]
mod tests;
