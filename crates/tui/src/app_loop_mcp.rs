//! Outcome handling for the `/mcp` modal, extracted from `app_loop.rs` to
//! mirror the `app_loop_model` extraction. The MCP handler is simpler than the
//! model handler: MCP config does not change the LLM endpoint, so it never
//! rebuilds the outer `client`. It only persists the JSON merge-patch, reloads
//! config, and notifies the session worker to pick up MCP changes.

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::mcp_menu::{handle_mcp_key, McpMenu, McpOutcome};
use crate::theme;
use crate::worker::UiCmd;

use super::LoopFlow;

/// Handle one keystroke while the `/mcp` modal is open. On `Save(json)`
/// persists the config patch, reloads it, sends `ReloadConfig` so the session
/// picks up the MCP server changes, and posts a confirmation marker. On
/// reload/save failure posts an error marker. `Cancel | Idle` does nothing.
/// Always returns [`LoopFlow::Proceed`] (the caller keeps its inline
/// `continue`).
pub(crate) async fn handle_mcp_outcome(
    mcp_menu: &mut Option<McpMenu>,
    k: KeyEvent,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    match handle_mcp_key(mcp_menu, k) {
        McpOutcome::Save(json) => match Config::save(workdir, &json) {
            Ok(path) => match Config::load(workdir) {
                Ok(reloaded) => {
                    *config = reloaded.clone();
                    let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                    chat.push_marker(Line::from(format!("[/mcp] saved → {}", path.display())));
                }
                Err(e) => {
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/mcp] reload failed: {e:#}"),
                        Style::default().fg(theme::err_color()),
                    )));
                }
            },
            Err(e) => {
                chat.push_marker(Line::from(Span::styled(
                    format!("[/mcp] save failed: {e:#}"),
                    Style::default().fg(theme::err_color()),
                )));
            }
        },
        McpOutcome::Cancel | McpOutcome::Idle => {}
    }
    LoopFlow::Proceed
}
