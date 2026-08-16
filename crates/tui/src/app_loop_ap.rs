//! Persistence and live-reload handling for the `/ap` modal (clone of
//! `app_loop_skill.rs`): the mode patch never changes the LLM endpoint, so
//! like `/cli`, `/mcp` and `/skill` it only persists, reloads and notifies
//! the worker.

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::ap_menu::{ApMenu, ApOutcome, handle_ap_key};
use crate::chat::ChatView;
use crate::worker::UiCmd;

use super::LoopFlow;

/// Handle one keystroke while the `/ap` modal is open. On `Save(json)`
/// persists the merge-patch, reloads config, sends `ReloadConfig` so the
/// worker honors the new autopilot mode at the next turn boundary, and
/// posts a purple `[ap] autopilot mode: ..` marker. Save/reload failure
/// posts a red error marker. Always `Proceed`.
pub(crate) async fn handle_ap_outcome(
    menu: &mut Option<ApMenu>,
    key: KeyEvent,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    if let ApOutcome::Save(json) = handle_ap_key(menu, key) {
        let mode = json["autopilot"]["mode"].as_str().unwrap_or("off").to_string();
        match Config::save(workdir, &json)
            .and_then(|path| Config::load(workdir).map(|cfg| (path, cfg)))
        {
            Ok((_path, reloaded)) => {
                *config = reloaded.clone();
                let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                chat.push_marker(Line::from(Span::styled(
                    format!("[ap] autopilot mode: {mode}"),
                    crate::theme::local_style(),
                )));
            }
            Err(error) => chat.push_marker(Line::from(Span::styled(
                format!("[ap] save/reload failed: {error:#}"),
                Style::default().fg(crate::theme::err_color()),
            ))),
        }
    }
    LoopFlow::Proceed
}
