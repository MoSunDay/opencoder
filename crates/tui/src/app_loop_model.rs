//! Model-outcome handling extracted from app_loop.rs to keep it under the 800-line cap.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::KeyEvent;
use opencoder_core::{Config, ConfigMode, ConfigSaveRoute};
use opencoder_llm::ChatStream;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::command::SlashAction;
use crate::model_menu::{
    handle_model_key, EnvMenu, ModeMenuKind, ModeSelection, ModelMenu, ModelOutcome,
};
use crate::model_session_switch::switch_session;
use crate::theme;
use crate::worker::{acknowledged_config_reload, ConfigReloadScope, UiCmd};

use super::LoopFlow;

fn saved_reload_error(chat: &mut ChatView, path: &Path, detail: &str) {
    chat.push_marker(Line::from(Span::styled(
        format!(
            "[/config] saved \u{2192} {}, but {detail}; disk changes were saved, live config was not refreshed",
            path.display()
        ),
        Style::default().fg(theme::err_color()),
    )));
}

/// The `match handle_model_key(...)` block from the `/config` modal: on
/// `Save(json)` persists config, sends an acknowledged `ReloadConfig`, then
/// rebuilds the outer client / config / model label / context limit / frame
/// ticker only after worker success and posts a marker. `Cancel | Idle` does
/// nothing. `Quit` sends `UiCmd::Quit` and was a `break`. Returns
/// [`LoopFlow::Quit`] for the `Quit` arm, otherwise [`LoopFlow::Proceed`] (the
/// caller keeps the post-match `continue` inline).
///
/// `running` is the safety net behind `gate_config_menu`: the three
/// worker-acknowledged outcomes are refused with a busy marker while a turn
/// runs (see [`worker_outcome_label`]) — the worker only acks between turns,
/// so awaiting it here would freeze the UI loop.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_model_outcome(
    model_menu: &mut Option<ModelMenu>,
    k: KeyEvent,
    client: &mut Arc<dyn ChatStream>,
    config: &mut Config,
    compaction_threshold: &mut u64,
    context_limit: &mut u64,
    frame_ms: &mut u64,
    frame_ticker: &mut tokio::time::Interval,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
    env_model: Option<&str>,
    running: bool,
) -> LoopFlow {
    let save_route = match model_menu.as_ref() {
        Some(ModelMenu::ModeConfig(form)) if form.mode == ConfigMode::Shared => {
            ConfigSaveRoute::Auto
        }
        Some(ModelMenu::List(_) | ModelMenu::Form(_) | ModelMenu::ModeConfig(_)) => {
            ConfigSaveRoute::EnvAware
        }
        _ => ConfigSaveRoute::Auto,
    };
    // An Env menu's pending command executes (disk mutation) inside the key
    // handler before any outcome exists — refuse before `handle_model_key`
    // consumes it, so the command stays pending for the idle drain.
    if running && matches!(model_menu, Some(ModelMenu::Env(menu)) if menu.has_pending()) {
        chat.push_marker(busy_marker("/env"));
        return LoopFlow::Proceed;
    }
    let outcome = handle_model_key(model_menu, k);
    if running {
        // Refuse, never park on the ack: the worker is inside `run_session`
        // and only drains commands (and acks) once the turn ends.
        if let Some(label) = worker_outcome_label(&outcome) {
            chat.push_marker(busy_marker(label));
            return LoopFlow::Proceed;
        }
    }
    match outcome {
        ModelOutcome::Save(json) => {
            let path = match Config::save_with_route(workdir, &json, save_route) {
                Ok(path) => path,
                Err(error) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/config] save failed: {error:#}"),
                        Style::default().fg(theme::err_color()),
                    )));
                    return LoopFlow::Proceed;
                }
            };
            let reloaded = match Config::load(workdir) {
                Ok(config) => config,
                Err(error) => {
                    saved_reload_error(chat, &path, &format!("config reload failed: {error:#}"));
                    return LoopFlow::Proceed;
                }
            };
            // Build the candidate without committing it. The worker performs
            // the same construction before its Store/config mutations.
            let new_client = match crate::model_session_switch::build_session_client(&reloaded) {
                Ok(client) => client,
                Err(error) => {
                    saved_reload_error(chat, &path, &format!("runtime reload failed: {error}"));
                    return LoopFlow::Proceed;
                }
            };
            let (command, mut completion) =
                acknowledged_config_reload(reloaded.clone(), ConfigReloadScope::Full);
            if let Err(error) = cmd_tx.send(command).await {
                saved_reload_error(chat, &path, &format!("reload dispatch failed: {error}"));
                return LoopFlow::Proceed;
            }
            match completion.recv().await {
                Some(Ok(())) => {}
                Some(Err(error)) => {
                    saved_reload_error(chat, &path, &format!("runtime reload failed: {error}"));
                    return LoopFlow::Proceed;
                }
                None => {
                    chat.push_marker(Line::from(Span::styled(
                        format!(
                            "[/config] saved \u{2192} {}, but runtime reload ended without confirmation; disk changes were saved, outer live config was not refreshed and worker state is unconfirmed",
                            path.display()
                        ),
                        Style::default().fg(theme::err_color()),
                    )));
                    return LoopFlow::Proceed;
                }
            }

            // Full saves update the durable Shared base but never displace an
            // existing session-only Shared selection. Empty metadata inherits
            // the newly reloaded Shared overlay.
            chat.shared_model =
                crate::model_scope::shared_model(&reloaded, chat.model_overrides.as_ref());
            *client = new_client;
            *compaction_threshold = reloaded.compaction.context_threshold;
            let new_frame_ms = reloaded.tui_frame_ms();
            if new_frame_ms != *frame_ms {
                *frame_ms = new_frame_ms;
                *frame_ticker = tokio::time::interval(Duration::from_millis(*frame_ms));
                frame_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            }
            let effective_model = reloaded.model.clone();
            *config = reloaded;
            chat.model = crate::model_scope::effective_for_chat(config, chat).model;
            *context_limit = crate::model_scope::effective_for_chat(config, chat).context_limit();
            chat.push_marker(Line::from(Span::styled(
                format!("[/config] saved \u{2192} {}", path.display()),
                Style::default().fg(theme::ok_color()),
            )));
            // Surface an exported model override instead of leaving the status
            // bar on a model different from the just-saved selection.
            if let Some(env_val) = env_model_override(
                json.get("model").and_then(|v| v.as_str()),
                &effective_model,
                env_model,
            ) {
                chat.push_marker(Line::from(Span::styled(
                    format!(
                        "[config] OPENCODER_MODEL is set ({env_val}) \u{2014} \
                         /model switch overridden by env"
                    ),
                    Style::default().fg(theme::err_color()),
                )));
            }
        }
        ModelOutcome::SaveSessionOnly(json) => {
            switch_session(json, config, client, cmd_tx, chat).await;
            *context_limit = crate::model_scope::effective_for_chat(config, chat).context_limit();
        }
        ModelOutcome::ReloadEnv(new_config, label) => {
            let reloaded = *new_config;
            let new_client = match crate::model_session_switch::build_session_client(&reloaded) {
                Ok(client) => client,
                Err(error) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/env] {label} failed: {error}; runtime unchanged"),
                        Style::default().fg(theme::err_color()),
                    )));
                    return LoopFlow::Proceed;
                }
            };
            let (ack, mut completion) = mpsc::unbounded_channel();
            if cmd_tx
                .send(UiCmd::ReloadEnv {
                    config: Box::new(reloaded.clone()),
                    ack,
                })
                .await
                .is_err()
            {
                chat.push_marker(Line::from(Span::styled(
                    format!(
                        "[/env] {label} failed: worker command channel closed; runtime unchanged"
                    ),
                    Style::default().fg(theme::err_color()),
                )));
                return LoopFlow::Proceed;
            }
            match completion.recv().await {
                Some(Ok(())) => {}
                Some(Err(error)) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/env] {label} failed: {error}; runtime unchanged"),
                        Style::default().fg(theme::err_color()),
                    )));
                    return LoopFlow::Proceed;
                }
                None => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/env] {label} failed: worker ended without confirmation; runtime unchanged"),
                        Style::default().fg(theme::err_color()),
                    )));
                    return LoopFlow::Proceed;
                }
            }
            *client = new_client;
            *compaction_threshold = reloaded.compaction.context_threshold;
            let new_frame_ms = reloaded.tui_frame_ms();
            if new_frame_ms != *frame_ms {
                *frame_ms = new_frame_ms;
                *frame_ticker = tokio::time::interval(Duration::from_millis(*frame_ms));
                frame_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            }
            *config = reloaded;
            chat.model_overrides = Some(opencoder_store::SessionModelOverrides::default());
            chat.shared_model =
                crate::model_scope::shared_model(config, chat.model_overrides.as_ref());
            chat.model = crate::model_scope::effective_for_chat(config, chat).model;
            *context_limit = crate::model_scope::effective_for_chat(config, chat).context_limit();
            chat.push_marker(Line::from(Span::styled(
                format!("[/env] {label}"),
                Style::default().fg(theme::ok_color()),
            )));
        }
        ModelOutcome::Cancel | ModelOutcome::Idle => {}
        ModelOutcome::Quit => {
            let _ = cmd_tx.send(UiCmd::Quit).await;
            return LoopFlow::Quit;
        }
    }
    LoopFlow::Proceed
}

/// Busy-turn refusal marker for a worker-acknowledged config command.
fn busy_marker(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("[{label}] busy \u{2014} change not applied, retry when idle"),
        Style::default().fg(theme::warn_color()),
    ))
}

/// Label of the slash command owning a worker-acknowledged outcome; `None`
/// for the outcomes that never talk to the worker (idle / cancel / quit) and
/// therefore stay available while a turn runs. Pure.
pub(crate) fn worker_outcome_label(outcome: &ModelOutcome) -> Option<&'static str> {
    match outcome {
        ModelOutcome::Save(_) => Some("/config"),
        ModelOutcome::SaveSessionOnly(_) => Some("/model"),
        ModelOutcome::ReloadEnv(..) => Some("/env"),
        ModelOutcome::Idle | ModelOutcome::Cancel | ModelOutcome::Quit => None,
    }
}

/// Open one of the worker-acknowledged config menus — the idle branch of the
/// `SlashAction::Model | Env | Config` dispatch (see `gate_config_menu`).
/// Pure menu construction: nothing reaches the worker until an outcome fires.
pub(crate) fn open_config_menu(
    action: SlashAction,
    model_menu: &mut Option<ModelMenu>,
    config: &Config,
    chat: &ChatView,
    agent_name: &str,
    workdir: &Path,
) {
    let mode_select = |kind: ModeMenuKind| {
        Some(ModelMenu::ModeSelect(Box::new(ModeSelection::new(
            crate::model_scope::menu_config(config, chat),
            crate::model_scope::mode_for_agent(agent_name),
            kind,
            workdir,
            chat.model_overrides.clone(),
        ))))
    };
    match action {
        SlashAction::Model => *model_menu = mode_select(ModeMenuKind::Model),
        SlashAction::Config => *model_menu = mode_select(ModeMenuKind::Config),
        SlashAction::Env(command) => {
            *model_menu = Some(ModelMenu::Env(EnvMenu::with_command(workdir, command)))
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn drain_pending_env(
    model_menu: &mut Option<ModelMenu>,
    client: &mut Arc<dyn ChatStream>,
    config: &mut Config,
    compaction_threshold: &mut u64,
    context_limit: &mut u64,
    frame_ms: &mut u64,
    frame_ticker: &mut tokio::time::Interval,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
    env_model: Option<&str>,
    running: bool,
) -> LoopFlow {
    let pending = matches!(model_menu, Some(ModelMenu::Env(menu)) if menu.has_pending());
    // While a turn runs the Env reload stays pending (silently — it is not a
    // keystroke) and drains at the first idle loop iteration.
    if !pending || running {
        return LoopFlow::Proceed;
    }
    handle_model_outcome(
        model_menu,
        KeyEvent::new(
            crossterm::event::KeyCode::Null,
            crossterm::event::KeyModifiers::NONE,
        ),
        client,
        config,
        compaction_threshold,
        context_limit,
        frame_ms,
        frame_ticker,
        cmd_tx,
        chat,
        workdir,
        env_model,
        running,
    )
    .await
}

/// Detect whether an exported `OPENCODER_MODEL` silently overrode a `/model`
/// switch. `Config::load` runs `apply_env` on every load, so an exported
/// `OPENCODER_MODEL` re-pins `cfg.model` and reverts a just-saved menu switch
/// -- leaving the status bar showing an unexpected model with no feedback.
///
/// Pure (no env I/O) so it is unit-testable without flaky process-wide env.
/// Returns the env model value when an override occurred, else `None`.
pub(crate) fn env_model_override(
    intended_model: Option<&str>,
    effective_model: &str,
    env_model: Option<&str>,
) -> Option<String> {
    let intended = intended_model?;
    let env = env_model?.trim();
    if env.is_empty() {
        return None;
    }
    (effective_model != intended).then(|| env.to_string())
}
