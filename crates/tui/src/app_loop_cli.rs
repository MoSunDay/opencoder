//! Persistence and live-reload handling for the `/cli` modal.

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::cli_menu::{handle_cli_key, CliMenu, CliOutcome};
use crate::worker::UiCmd;

use super::LoopFlow;

pub(crate) async fn handle_cli_outcome(
    menu: &mut Option<CliMenu>,
    key: KeyEvent,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    if let CliOutcome::Save(json) = handle_cli_key(menu, key) {
        match Config::save(workdir, &json)
            .and_then(|path| Config::load(workdir).map(|cfg| (path, cfg)))
        {
            Ok((path, reloaded)) => {
                *config = reloaded.clone();
                let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(reloaded))).await;
                chat.push_marker(Line::from(format!("[/cli] saved → {}", path.display())));
            }
            Err(error) => chat.push_marker(Line::from(Span::styled(
                format!("[/cli] save/reload failed: {error:#}"),
                Style::default().fg(crate::theme::err_color()),
            ))),
        }
    }
    LoopFlow::Proceed
}
