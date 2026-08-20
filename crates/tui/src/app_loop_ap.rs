//! Persistence handling for the `/ap` modal's confirm dialog (clone of
//! `app_loop_model.rs`'s `Save`/`SaveSessionOnly` split): Enter arms the
//! "save as default?" prompt, then `y` persists the mode globally while `n`
//! applies it session-only. The mode never changes the LLM endpoint, so no
//! client rebuild or `ReloadConfig` ever fires — both paths only notify the
//! worker via `ApModeSwitch` (which pins the session override and persists
//! `sessions.autopilot_mode`).

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::ap_menu::{ap_mode_json, handle_ap_key, ApMenu, ApOutcome};
use crate::chat::ChatView;
use crate::worker::UiCmd;

use super::LoopFlow;

/// Handle one keystroke while the `/ap` modal is open. `Save` (confirm `y`/
/// Enter) persists the merge-patch as the new global default, reloads the
/// config and notifies the worker; `SaveSessionOnly` (confirm `n`) merges
/// the mode into the in-memory config only and lets the worker pin the
/// session override + `sessions.autopilot_mode`. Both post a purple
/// `[ap] autopilot mode: ..` marker; save/reload failure posts a red error
/// marker. `Cancel | Idle` does nothing. Always `Proceed`.
pub(crate) async fn handle_ap_outcome(
    menu: &mut Option<ApMenu>,
    key: KeyEvent,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    match handle_ap_key(menu, key) {
        ApOutcome::Save(mode) => {
            // y: persist the merge-patch globally, reload, notify the worker
            // (which also pins this session's column, model-style).
            match Config::save(workdir, &ap_mode_json(mode))
                .and_then(|path| Config::load(workdir).map(|cfg| (path, cfg)))
            {
                Ok((_path, reloaded)) => {
                    *config = reloaded;
                    let _ = cmd_tx.send(UiCmd::ApModeSwitch(mode)).await;
                    chat.push_marker(Line::from(Span::styled(
                        format!("[ap] autopilot mode: {} (global default)", mode.as_str()),
                        crate::theme::local_style(),
                    )));
                }
                Err(error) => chat.push_marker(Line::from(Span::styled(
                    format!("[ap] save/reload failed: {error:#}"),
                    Style::default().fg(crate::theme::err_color()),
                ))),
            }
        }
        ApOutcome::SaveSessionOnly(mode) => {
            // n: merge into the in-memory config only; the worker pins the
            // session override and persists sessions.autopilot_mode.
            config.autopilot.mode = mode;
            let _ = cmd_tx.send(UiCmd::ApModeSwitch(mode)).await;
            chat.push_marker(Line::from(Span::styled(
                format!("[ap] autopilot mode: {} (session)", mode.as_str()),
                crate::theme::local_style(),
            )));
        }
        _ => {}
    }
    LoopFlow::Proceed
}
