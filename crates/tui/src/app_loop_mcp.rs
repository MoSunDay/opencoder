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
use serde_json::Value;
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::mcp_menu::patch::{colliding_server, normalized_server_name};
use crate::mcp_menu::{handle_mcp_key, McpMenu, McpOutcome};
use crate::theme;
use crate::worker::UiCmd;

use super::LoopFlow;

/// Extract the server keys an `mcp_servers` merge-patch touches: the key
/// being added/updated (the object-valued one) and the key being deleted
/// (the null-valued one — a rename's old name; pure deletes carry no object
/// key and return `None`). Pure; patch-shape knowledge lives here so the
/// collision guard below stays one-liner simple.
fn patch_server_keys(json: &Value) -> Option<(String, Option<String>)> {
    let servers = json.get("mcp_servers")?.as_object()?;
    let added = servers
        .iter()
        .find(|(_, v)| v.is_object())
        .map(|(k, _)| k.clone())?;
    let removed = servers
        .iter()
        .find(|(_, v)| v.is_null())
        .map(|(k, _)| k.clone());
    Some((added, removed))
}

/// Handle one keystroke while the `/mcp` modal is open. On `Save(json)`
/// persists the config patch, reloads it, sends `ReloadConfig` so the session
/// picks up the MCP server changes, and posts a confirmation marker. A save
/// whose server name collides (after normalization, bug #14) with a
/// differently-named configured server is refused before touching disk. On
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
        McpOutcome::Save(json) => {
            if let Some((added, removed)) = patch_server_keys(&json) {
                let existing: Vec<String> = config.mcp_servers.keys().cloned().collect();
                if let Some(conflict) = colliding_server(&added, removed.as_deref(), &existing) {
                    // Refuse, don't persist: `a-b` / `a.b` / `a_b` all
                    // normalize to the `mcp__a_b__…` tool prefix, and the
                    // registration map would silently overwrite one server's
                    // tools with the other's (plus `inject_to` scope leak).
                    chat.push_marker(Line::from(Span::styled(
                        format!(
                            "[/mcp] name conflict: `{added}` and `{conflict}` normalize to the \
                             same tool prefix `mcp__{}`; rename one of them",
                            normalized_server_name(&added)
                        ),
                        Style::default().fg(theme::err_color()),
                    )));
                    return LoopFlow::Proceed;
                }
            }
            match Config::save(workdir, &json) {
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
            }
        }
        McpOutcome::Cancel | McpOutcome::Idle => {}
    }
    LoopFlow::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatBlock, ChatView};
    use crate::mcp_menu::{McpField, McpForm};
    use crate::worker::UiCmd;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::config::McpServerConfig;
    use serde_json::json;

    /// Collect all marker-block text into a flat `String` for substring
    /// asserts (same helper shape as `app_loop_tests::mcp_outcome_tests`).
    fn marker_text(chat: &ChatView) -> String {
        chat.blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::Marker(lines) => Some(lines.as_slice()),
                _ => None,
            })
            .flat_map(|lines| lines.iter())
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn enter_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
    }

    /// A form whose only filled field is the server name — the minimal
    /// saveable shape (matches `mcp_outcome_tests::form_ready_to_save`).
    fn form_with_name(name: &str) -> McpMenu {
        let mut form = McpForm::new_blank();
        form.name = name.into();
        form.name_cursor = name.chars().count();
        form.field = McpField::Name;
        McpMenu::Form(form)
    }

    #[test]
    fn patch_server_keys_extracts_added_and_removed() {
        // Plain add: one object key, no delete marker.
        assert_eq!(
            patch_server_keys(&json!({"mcp_servers": {"c": {"enabled": true}}})),
            Some(("c".into(), None))
        );
        // Rename: new object key + null delete marker for the old key.
        assert_eq!(
            patch_server_keys(&json!({"mcp_servers": {"a-b": {"enabled": true}, "a.b": null}})),
            Some(("a-b".into(), Some("a.b".into())))
        );
        // Pure delete: no object key → nothing to collision-check.
        assert_eq!(
            patch_server_keys(&json!({"mcp_servers": {"old": null}})),
            None
        );
        // Not an mcp_servers patch at all.
        assert_eq!(patch_server_keys(&json!({"model": "x"})), None);
    }

    /// Bug #14 regression: saving `a.b` while `a-b` is configured would
    /// silently shadow its tools at registration time (both normalize to
    /// `mcp__a_b__…`). The save must be refused before `Config::save` runs:
    /// no domain file is written, the config object is untouched, no
    /// `ReloadConfig` is dispatched, and a red error marker names both sides.
    #[tokio::test]
    async fn handle_mcp_outcome_refuses_save_colliding_after_normalization() {
        let tmp = tempfile::tempdir().unwrap();
        let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
        let workdir = tmp.path();

        let mut mcp_menu = Some(form_with_name("a.b"));
        let mut config = Config::default();
        config
            .mcp_servers
            .insert("a-b".to_string(), McpServerConfig::default());
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
        let mut chat = ChatView::default();

        let flow = handle_mcp_outcome(
            &mut mcp_menu,
            enter_key(),
            &mut config,
            &cmd_tx,
            &mut chat,
            workdir,
        )
        .await;

        assert!(matches!(flow, LoopFlow::Proceed));
        let text = marker_text(&chat);
        assert!(
            text.contains("name conflict")
                && text.contains("a.b")
                && text.contains("a-b")
                && text.contains("mcp__a_b"),
            "expected conflict marker naming both servers, got: {text}"
        );
        // Refused before persisting: neither domain file nor config.json
        // may exist under the isolated home / project dir.
        assert!(!tmp.path().join(".opencoder").join("mcp.json").exists());
        assert!(!workdir.join("opencoder.json").exists());
        // In-memory config unchanged and worker not told to reload.
        assert!(!config.mcp_servers.contains_key("a.b"));
        assert!(
            cmd_rx.try_recv().is_err(),
            "ReloadConfig must not be sent on a refused save"
        );
    }

    /// The guard must not over-block: a save whose normalized name is
    /// genuinely distinct from every configured server persists as before.
    #[tokio::test]
    async fn handle_mcp_outcome_allows_non_colliding_save_alongside_similar_name() {
        let tmp = tempfile::tempdir().unwrap();
        let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
        let workdir = tmp.path();

        let mut mcp_menu = Some(form_with_name("c-d"));
        let mut config = Config::default();
        config
            .mcp_servers
            .insert("a-b".to_string(), McpServerConfig::default());
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
        let mut chat = ChatView::default();

        handle_mcp_outcome(
            &mut mcp_menu,
            enter_key(),
            &mut config,
            &cmd_tx,
            &mut chat,
            workdir,
        )
        .await;

        assert!(marker_text(&chat).contains("[/mcp] saved"));
        assert!(matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))));
        assert!(config.mcp_servers.contains_key("c-d"));
        // And it landed in the global domain file (no project mcp.json).
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join(".opencoder").join("mcp.json")).unwrap(),
        )
        .unwrap();
        assert!(saved.get("c-d").is_some(), "saved = {saved}");
        assert!(saved.get("a-b").is_none(), "saved = {saved}");
    }
}
