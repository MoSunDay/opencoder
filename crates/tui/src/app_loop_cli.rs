//! `/cli` outcome handling extracted from `app_loop.rs`.

use std::path::Path;

use crossterm::event::KeyEvent;
use opencoder_core::{Config, ConfigSaveRoute};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::cli_menu::{handle_cli_key, CliMenu, CliOutcome};
use crate::worker::{acknowledged_config_reload, ConfigReloadScope, UiCmd};

use super::LoopFlow;

fn saved_reload_error(chat: &mut ChatView, path: &Path, detail: &str) {
    chat.push_marker(Line::from(Span::styled(
        format!(
            "[/cli] saved \u{2192} {}, but {detail}; disk changes were saved, live config was not refreshed",
            path.display()
        ),
        Style::default().fg(crate::theme::err_color()),
    )));
}

/// Persist a CLI-domain edit, then reload only that domain in the worker.
/// Provider/model/mode runtime state and the ChatView/client stay untouched.
pub(crate) async fn handle_cli_outcome(
    menu: &mut Option<CliMenu>,
    key: KeyEvent,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    if let CliOutcome::Save(json) = handle_cli_key(menu, key) {
        let path = match Config::save_with_route(workdir, &json, ConfigSaveRoute::EnvAware) {
            Ok(path) => path,
            Err(error) => {
                chat.push_marker(Line::from(Span::styled(
                    format!("[/cli] save failed: {error:#}"),
                    Style::default().fg(crate::theme::err_color()),
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
        let mut candidate = config.clone();
        candidate.cli = reloaded.cli.clone();
        let (command, mut completion) =
            acknowledged_config_reload(candidate, ConfigReloadScope::Cli);
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
                        "[/cli] saved \u{2192} {}, but runtime reload ended without confirmation; disk changes were saved, outer live config was not refreshed and worker state is unconfirmed",
                        path.display()
                    ),
                    Style::default().fg(crate::theme::err_color()),
                )));
                return LoopFlow::Proceed;
            }
        }
        config.cli = reloaded.cli;
        chat.push_marker(Line::from(format!(
            "[/cli] saved \u{2192} {}",
            path.display()
        )));
    }
    LoopFlow::Proceed
}
