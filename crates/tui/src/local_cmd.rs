//! Display-only, context-free slash commands (`/ps`, `/stop`).
//!
//! These commands inspect / mutate the *local* background-bash registry,
//! rendering their result directly into the transcript as a purple
//! marker. They never call `record()`, never start a turn, and never reach
//! `session.messages` — they are purely user-facing chrome. The popup is the
//! primary entry path (`/` opens the picker); an idle free-text/paste path
//! (`app.rs` Submit) is the fallback.
//!
//! `/ap` used to live here as a direct on/off toggle; it now opens the
//! `ap_menu` tri-state picker instead (see `crate::ap_menu`).

use ratatui::text::{Line, Span};

use opencoder_session::tools::bg;

use crate::chat::ChatView;
use crate::theme;

/// Fixed message shown after `/stop`.
const STOP_MESSAGE: &str = "Process has been terminated.";

/// Run a local command if `text` is recognised. Returns `true` when handled
/// (a purple marker has been pushed to `chat`), `false` otherwise so the
/// caller falls through to the normal submit path.
pub(crate) async fn run(text: &str, chat: &mut ChatView) -> bool {
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
        _ => return false,
    }
    true
}

/// Whether `text` (trimmed, optional leading `/`) names a local command.
pub(crate) fn is_local(text: &str) -> bool {
    matches!(text.trim().trim_start_matches('/'), "ps" | "stop")
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
        assert!(is_local("ps"));
        assert!(is_local("stop"));
        assert!(is_local("  /ps  "));
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

    /// A non-local command must not touch chat: `run` falls through.
    #[tokio::test]
    async fn run_unknown_falls_through() {
        let mut chat = ChatView::default();
        assert!(!run("hello", &mut chat).await);
        assert!(chat.blocks.is_empty(), "no marker pushed for non-local");
    }

    /// Flatten a `Line` into its plain string for assertions.
    fn render(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }
}
