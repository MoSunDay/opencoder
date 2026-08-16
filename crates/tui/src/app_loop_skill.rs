//! Persistence and live-reload handling for the `/skill` modal (clone of
//! `app_loop_cli.rs`): the toggle patch never changes the LLM endpoint, so
//! like `/cli` and `/mcp` it only persists, reloads and notifies the worker.

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::skill_menu::{SkillMenu, SkillOutcome, handle_skill_key};
use crate::worker::UiCmd;

use super::LoopFlow;

/// Handle one keystroke while the `/skill` modal is open. On `Save(json)`
/// persists the merge-patch, reloads config, sends `ReloadConfig` so the
/// session picks up the new default-injection set, and posts a marker.
/// Save/reload failure posts a red error marker. Always `Proceed`.
pub(crate) async fn handle_skill_outcome(
    menu: &mut Option<SkillMenu>,
    key: KeyEvent,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    if let SkillOutcome::Save(json) = handle_skill_key(menu, key) {
        match Config::save(workdir, &json)
            .and_then(|path| Config::load(workdir).map(|cfg| (path, cfg)))
        {
            Ok((path, reloaded)) => {
                *config = reloaded.clone();
                let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                chat.push_marker(Line::from(format!("[/skill] saved → {}", path.display())));
            }
            Err(error) => chat.push_marker(Line::from(Span::styled(
                format!("[/skill] save/reload failed: {error:#}"),
                Style::default().fg(crate::theme::err_color()),
            ))),
        }
    }
    LoopFlow::Proceed
}
