//! Session-only model switch helper.
//!
//! `ModelOutcome::SaveSessionOnly` applies a model change to the in-memory
//! config + hot-swaps the LLM client **without** writing `opencoder.json`.
//! The worker still persists the new model to the session store row (resume
//! honors it) because a `ReloadConfig` command is dispatched.

use std::sync::Arc;

use opencoder_core::Config;
use opencoder_llm::ChatStream;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::theme;
use crate::worker::UiCmd;

/// Apply a session-only model switch: update the in-memory `config`, rebuild
/// the outer `client` so new `/task` sessions pick up the endpoint, dispatch
/// `ReloadConfig` (worker persists to the session store row), and push a cyan
/// "switched (session only)" marker. `opencoder.json` is **not** touched.
pub(crate) async fn switch_session(
    json: serde_json::Value,
    config: &mut Config,
    client: &mut Arc<dyn ChatStream>,
    cmd_tx: &mpsc::Sender<UiCmd>,
    chat: &mut ChatView,
) {
    if let Some(m) = json.get("model").and_then(|v| v.as_str()) {
        config.model = m.to_string();
    }
    let new_config = config.clone();
    match new_config.resolve_endpoint() {
        Ok(ep) => match opencoder_llm::ChatClient::new_with_read_timeout(
            &ep.base_url,
            &ep.api_key,
            &ep.headers,
            new_config.stream_idle_timeout(),
            new_config.network.proxy.as_deref(),
        ) {
            Ok(new_client) => {
                *client = Arc::new(new_client);
            }
            Err(e) => {
                chat.push_marker(Line::from(Span::styled(
                    format!(
                        "[/model] client build failed: {e:#} \u{2014}                         live session keeps previous client"
                    ),
                    Style::default().fg(theme::err_color()),
                )));
            }
        },
        Err(e) => {
            chat.push_marker(Line::from(Span::styled(
                format!("[/model] endpoint resolve failed: {e:#}"),
                Style::default().fg(theme::err_color()),
            )));
        }
    }
    let _ = cmd_tx.send(UiCmd::ReloadConfig(Box::new(new_config))).await;
    chat.push_marker(Line::from(Span::styled(
        format!(
            "[/model] switched (session only) \u{2192} {}",
            config.model_id()
        ),
        Style::default().fg(theme::accent()),
    )));
}
