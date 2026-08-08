//! Display-only, context-free slash commands (`/ps`, `/stop`, `/ap`).
//!
//! These commands inspect / mutate the *local* background-bash registry and
//! config, rendering their result directly into the transcript as a purple
//! marker. They never call `record()`, never start a turn, and never reach
//! `session.messages` — they are purely user-facing chrome. The popup is the
//! primary entry path (`/` opens the picker); an idle free-text/paste path
//! (`app.rs` Submit) is the fallback.
//!
//! `/ap` is the one command with a side effect beyond chrome: it flips
//! `autopilot.enabled` in the on-disk config (`Config::save` + reload) and
//! forwards `UiCmd::ReloadConfig` so the worker honors it at the next turn
//! boundary. The marker still reflects the target state immediately.

use std::path::Path;

use opencoder_core::Config;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;

use opencoder_session::tools::bg;

use crate::chat::ChatView;
use crate::theme;
use crate::worker::UiCmd;

/// Fixed message shown after `/stop`.
const STOP_MESSAGE: &str = "Process has been terminated.";

/// Run a local command if `text` is recognised. Returns `true` when handled
/// (a purple marker has been pushed to `chat`), `false` otherwise so the
/// caller falls through to the normal submit path.
pub(crate) async fn run(
    text: &str,
    chat: &mut ChatView,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    workdir: &Path,
) -> bool {
    if !is_local(text) {
        return false;
    }
    let bare = text.trim().trim_start_matches('/');
    match bare {
        "ps" => chat.push_marker_lines(format_ps(&bg::list())),
        "stop" => {
            bg::kill_all();
            chat.push_marker_lines(vec![Line::from(Span::styled(
                format!("[stop] {STOP_MESSAGE}"),
                theme::local_style(),
            ))]);
        }
        "ap" => toggle_ap(chat, config, cmd_tx, workdir).await,
        _ => return false,
    }
    true
}

/// Whether `text` (trimmed, optional leading `/`) names a local command.
pub(crate) fn is_local(text: &str) -> bool {
    matches!(text.trim().trim_start_matches('/'), "ps" | "stop" | "ap")
}

/// Flip `autopilot.enabled` on disk, reload, and re-broadcast the config to
/// the worker (`UiCmd::ReloadConfig`) so the next turn boundary honors it.
/// Pushes a purple `[ap] autopilot: on|off` marker on success; on save/reload
/// failure a red marker mirroring the `[/config]` error style.
async fn toggle_ap(
    chat: &mut ChatView,
    config: &mut Config,
    cmd_tx: &mpsc::Sender<UiCmd>,
    workdir: &Path,
) {
    let next = !config.autopilot.enabled;
    let patch = serde_json::json!({ "autopilot": { "enabled": next } });
    match Config::save(workdir, &patch).and_then(|_| Config::load(workdir)) {
        Ok(reloaded) => {
            *config = reloaded.clone();
            if cmd_tx
                .send(UiCmd::ReloadConfig(Box::new(reloaded)))
                .await
                .is_err()
            {
                chat.push_marker(Line::from(Span::styled(
                    "[ap] worker channel closed — config saved but not applied",
                    Style::default().fg(theme::err_color()),
                )));
                return;
            }
            chat.push_marker(Line::from(Span::styled(
                ap_marker_text(next),
                theme::local_style(),
            )));
        }
        Err(e) => {
            chat.push_marker(Line::from(Span::styled(
                format!("[ap] save failed: {e:#}"),
                Style::default().fg(theme::err_color()),
            )));
        }
    }
}

/// Pure formatting of a `/ps` snapshot into purple marker lines.
fn format_ps(procs: &[bg::BgInfo]) -> Vec<Line<'static>> {
    if procs.is_empty() {
        return vec![Line::from(Span::styled(
            "[ps] no background processes",
            theme::local_style(),
        ))];
    }
    let mut lines = Vec::with_capacity(procs.len() + 1);
    lines.push(Line::from(Span::styled(
        format!("[ps] background bash ({}):", procs.len()),
        theme::local_style(),
    )));
    for p in procs {
        lines.push(Line::from(Span::styled(
            format!("  {}  {}", p.pid, p.output_path.display()),
            theme::local_style(),
        )));
    }
    lines
}

/// Pure formatting of the `/ap` marker text for the given target state.
fn ap_marker_text(enabled: bool) -> String {
    format!("[ap] autopilot: {}", if enabled { "on" } else { "off" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn info(pid: u32) -> bg::BgInfo {
        bg::BgInfo {
            pid,
            output_path: PathBuf::from(format!("/tmp/opencoder_bg_{pid}.output")),
        }
    }

    #[test]
    fn format_ps_empty() {
        let lines = format_ps(&[]);
        assert_eq!(lines.len(), 1);
        let rendered = render(&lines[0]);
        assert_eq!(rendered, "[ps] no background processes");
    }

    #[test]
    fn format_ps_one() {
        let lines = format_ps(&[info(111)]);
        assert_eq!(lines.len(), 2);
        assert_eq!(render(&lines[0]), "[ps] background bash (1):");
        assert_eq!(render(&lines[1]), "  111  /tmp/opencoder_bg_111.output");
    }

    #[test]
    fn format_ps_many() {
        let lines = format_ps(&[info(1), info(22), info(333)]);
        assert_eq!(lines.len(), 4);
        assert_eq!(render(&lines[0]), "[ps] background bash (3):");
        assert_eq!(render(&lines[1]), "  1  /tmp/opencoder_bg_1.output");
        assert_eq!(render(&lines[2]), "  22  /tmp/opencoder_bg_22.output");
        assert_eq!(render(&lines[3]), "  333  /tmp/opencoder_bg_333.output");
    }

    #[test]
    fn is_local_matches() {
        assert!(is_local("/ps"));
        assert!(is_local("/stop"));
        assert!(is_local("/ap"));
        assert!(is_local("ps"));
        assert!(is_local("stop"));
        assert!(is_local("ap"));
        assert!(is_local("  /ps  "));
        assert!(is_local(" /ap "));
    }

    #[test]
    fn is_local_non_matches() {
        assert!(!is_local("/task"));
        assert!(!is_local("/pss"));
        assert!(!is_local("hello"));
        assert!(!is_local(""));
        assert!(!is_local("/"));
    }

    #[test]
    fn stop_message_text() {
        assert_eq!(STOP_MESSAGE, "Process has been terminated.");
    }

    #[test]
    fn ap_marker_text_reflects_target_state() {
        assert_eq!(ap_marker_text(true), "[ap] autopilot: on");
        assert_eq!(ap_marker_text(false), "[ap] autopilot: off");
    }

    /// `/ap` persists the toggle through a real `Config::save` + `Config::load`
    /// round-trip in a tempdir: on → off → on, asserting the on-disk JSON and
    /// the in-memory config agree after each flip, and that a reload command
    /// was dispatched every time.
    #[tokio::test]
    async fn toggle_ap_round_trips_persisted_state() {
        // Isolate config discovery to a tempdir on this thread only — no
        // process-env mutation, so no `set_var` UB and no race with the
        // sys_tokens_* readers elsewhere in the test binary.
        let fake_home = tempfile::tempdir().expect("tempdir for fake config home");
        let _iso = opencoder_core::scoped_config_home(fake_home.path().to_path_buf());
        let dir = tempfile::tempdir().expect("tempdir");
        let workdir = dir.path();
        let mut config = Config::default();
        assert!(!config.autopilot.enabled, "precondition: default off");

        let mut chat = ChatView::default();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);

        // First toggle: off -> on.
        toggle_ap(&mut chat, &mut config, &cmd_tx, workdir).await;
        assert!(config.autopilot.enabled, "in-memory config flipped to on");
        assert_eq!(
            ap_marker_text(true),
            marker_text(&chat),
            "purple marker reflects the new state"
        );
        let cmd = cmd_rx.try_recv().expect("ReloadConfig must be dispatched");
        assert!(matches!(cmd, UiCmd::ReloadConfig(_)));
        // On-disk JSON carries enabled=true (deep merge must not have dropped it).
        let raw = std::fs::read_to_string(workdir.join("opencoder.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()["autopilot"]["enabled"],
            serde_json::json!(true),
            "enabled persisted to disk"
        );

        // Reload from disk: the toggle survives a full Config::load.
        let reloaded = Config::load(workdir).unwrap();
        assert!(reloaded.autopilot.enabled, "reload sees enabled=true");

        // Second toggle: on -> off.
        toggle_ap(&mut chat, &mut config, &cmd_tx, workdir).await;
        assert!(
            !config.autopilot.enabled,
            "in-memory config flipped back to off"
        );
        assert_eq!(ap_marker_text(false), marker_text(&chat));
        let cmd = cmd_rx.try_recv().expect("second ReloadConfig dispatched");
        assert!(matches!(cmd, UiCmd::ReloadConfig(_)));
        let raw = std::fs::read_to_string(workdir.join("opencoder.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()["autopilot"]["enabled"],
            serde_json::json!(false),
            "enabled persisted back to false"
        );
    }

    /// `/ap` on a non-local command must not touch config or chat.
    #[tokio::test]
    async fn run_unknown_falls_through() {
        let mut chat = ChatView::default();
        let mut config = Config::default();
        let (cmd_tx, _cmd_rx) = mpsc::channel(8);
        assert!(!run("hello", &mut chat, &mut config, &cmd_tx, Path::new(".")).await);
        assert!(chat.blocks.is_empty(), "no marker pushed for non-local");
    }

    /// The latest marker line of the chat transcript as plain text.
    fn marker_text(chat: &ChatView) -> String {
        let blocks = &chat.blocks;
        let last = blocks.last().expect("a marker was pushed");
        match last {
            crate::chat::ChatBlock::Marker(lines) => lines
                .last()
                .expect("marker has at least one line")
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect(),
            other => panic!("expected Marker block, got {other:?}"),
        }
    }

    /// Flatten a `Line` into its plain string for assertions.
    fn render(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}
