//! Model-outcome handling extracted from app_loop.rs to keep it under the 800-line cap.

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
use crate::model_menu::{handle_model_key, ModelMenu, ModelOutcome};
use crate::model_session_switch::switch_session;
use crate::theme;
use crate::worker::UiCmd;

use super::env_model_override;
use super::LoopFlow;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_model_outcome(
    model_menu: &mut Option<ModelMenu>,
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
    match handle_model_key(model_menu, k) {
        ModelOutcome::Save(json) => {
            match Config::save(workdir, &json) {
                Ok(path) => {
                    match Config::load(workdir) {
                        Ok(reloaded) => {
                            *model_label = reloaded.model.clone();
                            *compaction_threshold = reloaded.compaction.context_threshold;
                            *context_limit = reloaded.context_limit();
                            // Rebuild the outer `client` too so subsequent
                            // `/task` new sessions pick up the new endpoint
                            // (the worker only swaps its own sess.client).
                            match reloaded.resolve_endpoint() {
                                Ok(ep) => match opencoder_llm::ChatClient::new_with_read_timeout(
                                    &ep.base_url,
                                    &ep.api_key,
                                    &ep.headers,
                                    reloaded.stream_idle_timeout(),
                                    reloaded.network.proxy.as_deref(),
                                ) {
                                    Ok(new_client) => {
                                        *client = Arc::new(new_client);
                                    }
                                    Err(e) => {
                                        chat.push_marker(Line::from(Span::styled(
                                            format!(
                                                "[/config] client build failed: {e:#} — \
                                                 live session keeps previous client"
                                            ),
                                            Style::default().fg(theme::err_color()),
                                        )));
                                    }
                                },
                                Err(e) => {
                                    chat.push_marker(Line::from(Span::styled(
                                        format!(
                                            "[/config] endpoint resolve failed: {e:#} — \
                                             live session keeps previous client"
                                        ),
                                        Style::default().fg(theme::err_color()),
                                    )));
                                }
                            }
                            crate::theme::set_theme(crate::theme::ThemeKind::from_label(&reloaded.theme));
                            *config = reloaded.clone();
                            let effective_model = reloaded.model.clone();
                            // Apply a new TUI frame rate immediately: rebuild the frame
                            // interval so the just-saved fps takes effect without restart.
                            let new_frame_ms = reloaded.tui_frame_ms();
                            if new_frame_ms != *frame_ms {
                                *frame_ms = new_frame_ms;
                                *frame_ticker =
                                    tokio::time::interval(Duration::from_millis(*frame_ms));
                                frame_ticker.set_missed_tick_behavior(
                                    tokio::time::MissedTickBehavior::Skip,
                                );
                            }
                            let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                            chat.push_marker(Line::from(Span::styled(
                                format!("[/config] saved \u{2192} {}", path.display()),
                                Style::default().fg(theme::ok_color()),
                            )));
                            // Issue #2: if an exported OPENCODER_MODEL silently
                            // reverted this /model switch, surface it instead of
                            // leaving the status bar on an unexpected model.
                            if let Some(env_val) = env_model_override(
                                json.get("model").and_then(|v| v.as_str()),
                                &effective_model,
                                std::env::var("OPENCODER_MODEL").ok().as_deref(),
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
                        Err(e) => {
                            chat.push_marker(Line::from(Span::styled(
                                format!("[/config] reload failed: {e:#}"),
                                Style::default().fg(theme::err_color()),
                            )));
                        }
                    }
                }
                Err(e) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/config] save failed: {e:#}"),
                        Style::default().fg(theme::err_color()),
                    )));
                }
            }
        }
        ModelOutcome::SaveSessionOnly(json) => {
            switch_session(json, config, client, cmd_tx, chat).await;
            *model_label = config.model.clone();
            *compaction_threshold = config.compaction.context_threshold;
            *context_limit = config.context_limit();
        }
        ModelOutcome::Cancel | ModelOutcome::Idle => {}
        ModelOutcome::Quit => {
            let _ = cmd_tx.send(UiCmd::Quit).await;
            return LoopFlow::Quit;
        }
    }
    LoopFlow::Proceed
}
