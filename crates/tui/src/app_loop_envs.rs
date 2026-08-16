//! `/envs` outcome handling: executes env mutations against the core envs API
//! and mirrors the `/model` full-refresh path (env config may change
//! model/provider/theme/fps: rebuild outer client, labels, theme, ticker)
//! followed by `UiCmd::ReloadConfig` for the session worker.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use opencoder_llm::ChatStream;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::envs_menu::{handle_envs_key, EnvsList, EnvsMenu, EnvsOutcome};
use crate::theme;
use crate::worker::UiCmd;

use super::LoopFlow;

fn err_marker(chat: &mut ChatView, msg: String) {
    chat.push_marker(Line::from(Span::styled(
        format!("[/envs] {msg}"),
        Style::default().fg(theme::err_color()),
    )));
}

/// Handle one keystroke while the `/envs` modal is open. Mutating outcomes
/// run against `~/.opencoder/envs/`; anything that can change the effective
/// config (activate/deactivate, recapture/delete of the active env) runs the
/// full `/model`-style refresh. List-only mutations (create/delete/recapture
/// of a non-active env) keep the modal open with fresh state.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_envs_outcome(
    envs_menu: &mut Option<EnvsMenu>,
    k: KeyEvent,
    client: &mut Arc<dyn ChatStream>,
    config: &mut Config,
    model_label: &mut String,
    compaction_threshold: &mut u64,
    context_limit: &mut u64,
    frame_ms: &mut u64,
    frame_ticker: &mut tokio::time::Interval,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    match handle_envs_key(envs_menu, k) {
        EnvsOutcome::Idle => {}
        EnvsOutcome::Cancel => *envs_menu = None,
        EnvsOutcome::Activate(name) => {
            match opencoder_core::set_active_env(Some(&name)) {
                Ok(()) => {
                    *envs_menu = None;
                    refresh_after_env_change(
                        format!("activated \u{2192} {name}"),
                        client,
                        config,
                        model_label,
                        compaction_threshold,
                        context_limit,
                        frame_ms,
                        frame_ticker,
                        cmd_tx,
                        chat,
                        workdir,
                    )
                    .await;
                }
                Err(e) => {
                    err_marker(chat, format!("activate failed: {e}"));
                    *envs_menu = Some(EnvsMenu::List(EnvsList::discover()));
                }
            }
        }
        EnvsOutcome::Deactivate => match opencoder_core::set_active_env(None) {
            Ok(()) => {
                *envs_menu = None;
                refresh_after_env_change(
                    "deactivated \u{2192} base config".to_string(),
                    client,
                    config,
                    model_label,
                    compaction_threshold,
                    context_limit,
                    frame_ms,
                    frame_ticker,
                    cmd_tx,
                    chat,
                    workdir,
                )
                .await;
            }
            Err(e) => {
                err_marker(chat, format!("deactivate failed: {e}"));
                *envs_menu = Some(EnvsMenu::List(EnvsList::discover()));
            }
        },
        EnvsOutcome::Create { name, capture } => {
            match opencoder_core::create_env(&name, workdir, capture) {
                Ok(dir) => chat.push_marker(Line::from(Span::styled(
                    format!("[/envs] created \u{2192} {}", dir.display()),
                    Style::default().fg(theme::ok_color()),
                ))),
                Err(e) => err_marker(chat, format!("create failed: {e:#}")),
            }
            *envs_menu = Some(EnvsMenu::List(EnvsList::discover()));
        }
        EnvsOutcome::Recapture(name) => {
            let was_active = opencoder_core::active_env().as_deref() == Some(name.as_str());
            match opencoder_core::recapture_env(&name, workdir) {
                Ok(()) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/envs] recaptured \u{2192} {name}"),
                        Style::default().fg(theme::ok_color()),
                    )));
                    if was_active {
                        refresh_after_env_change(
                            format!("recaptured active env {name}"),
                            client,
                            config,
                            model_label,
                            compaction_threshold,
                            context_limit,
                            frame_ms,
                            frame_ticker,
                            cmd_tx,
                            chat,
                            workdir,
                        )
                        .await;
                    }
                }
                Err(e) => err_marker(chat, format!("recapture failed: {e:#}")),
            }
            *envs_menu = Some(EnvsMenu::List(EnvsList::discover()));
        }
        EnvsOutcome::Delete(name) => {
            let was_active = opencoder_core::active_env().as_deref() == Some(name.as_str());
            match opencoder_core::delete_env(&name) {
                Ok(()) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/envs] deleted {name}"),
                        Style::default().fg(theme::ok_color()),
                    )));
                    if was_active {
                        refresh_after_env_change(
                            format!("deleted active env {name} \u{2192} base config"),
                            client,
                            config,
                            model_label,
                            compaction_threshold,
                            context_limit,
                            frame_ms,
                            frame_ticker,
                            cmd_tx,
                            chat,
                            workdir,
                        )
                        .await;
                    }
                }
                Err(e) => err_marker(chat, format!("delete failed: {e:#}")),
            }
            *envs_menu = Some(EnvsMenu::List(EnvsList::discover()));
        }
    }
    LoopFlow::Proceed
}

/// The `/model` refresh sequence minus the save (the env marker is already
/// written): reload config, rebuild the outer client, apply labels/theme/fps,
/// then notify the session worker via `ReloadConfig`.
#[allow(clippy::too_many_arguments)]
async fn refresh_after_env_change(
    tag: String,
    client: &mut Arc<dyn ChatStream>,
    config: &mut Config,
    model_label: &mut String,
    compaction_threshold: &mut u64,
    context_limit: &mut u64,
    frame_ms: &mut u64,
    frame_ticker: &mut tokio::time::Interval,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) {
    match Config::load(workdir) {
        Ok(reloaded) => {
            *model_label = reloaded.model.clone();
            *compaction_threshold = reloaded.compaction.context_threshold;
            *context_limit = reloaded.context_limit();
            // Rebuild the outer client too so subsequent prompts use the
            // env's endpoint immediately (mirrors /model).
            match reloaded.resolve_endpoint() {
                Ok(ep) => match opencoder_llm::ChatClient::new_with_read_timeout(
                    &ep.base_url,
                    &ep.api_key,
                    &ep.headers,
                    reloaded.stream_idle_timeout(),
                    reloaded.network.proxy.as_deref(),
                ) {
                    Ok(new_client) => *client = Arc::new(new_client),
                    Err(e) => err_marker(
                        chat,
                        format!("client build failed: {e:#} \u{2014} live session keeps previous client"),
                    ),
                },
                Err(e) => err_marker(
                    chat,
                    format!("endpoint resolve failed: {e:#} \u{2014} live session keeps previous client"),
                ),
            }
            crate::theme::set_theme(crate::theme::ThemeKind::from_label(&reloaded.theme));
            *config = reloaded.clone();
            let new_frame_ms = reloaded.tui_frame_ms();
            if new_frame_ms != *frame_ms {
                *frame_ms = new_frame_ms;
                *frame_ticker = tokio::time::interval(Duration::from_millis(new_frame_ms));
                frame_ticker
                    .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            }
            let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
            chat.push_marker(Line::from(Span::styled(
                format!("[/envs] {tag}"),
                Style::default().fg(theme::ok_color()),
            )));
        }
        Err(e) => err_marker(chat, format!("reload failed: {e:#}")),
    }
}
