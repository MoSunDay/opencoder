//! Display-state helpers extracted from `app.rs` to keep the event loop concise.
//!
//! Besides the steer/queue/timer helpers this module owns the top body-title
//! *composition* (pure, terminal-free): `workdir · model · effort`.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::chat::{ChatBlock, ChatView};
use crate::theme;

/// Display width (terminal columns) of the jump-to-top `⬆` arrow label on the
/// body's top-border row: `"    ⬆    "` (4 spaces + wide ⬆ + 4 spaces). Must
/// stay in sync with the literal rendered in `render.rs` (guarded by a test).
pub(super) const TOP_ARROW_W: u16 = 10;

/// Compose the top-level body title `Line` for the non-subagent view.
///
/// Graded palette (harmonised with the bottom-border corners): the static
/// `workdir` uses the bold bright-blue status-label colour (same as the
/// status bar's `thr` prefix), the `·` separators nearly vanish
/// (muted), the model carries the cyan accent (matching the `[tok cost]`
/// corner / follow indicator), and the thinking level takes the pink
/// reserved for the Thinking block header. The mode is rendered at the
/// bottom-left in the status bar.
pub(super) fn compose_top_title(
    workdir: &Path,
    model_bare: &str,
    effort: Option<&str>,
) -> Line<'static> {
    let workdir =
        crate::terminal_text::sanitize_single_line(&workdir.display().to_string()).into_owned();
    let model_bare = crate::terminal_text::sanitize_single_line(model_bare).into_owned();
    let effort = effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::terminal_text::sanitize_single_line)
        .map(|value| value.into_owned());

    let thr_label = theme::bold(theme::status_label_color());
    let muted = Style::default().fg(theme::muted());
    let accent = Style::default().fg(theme::accent());
    let pink = Style::default().fg(theme::pink());

    let mut spans = vec![
        Span::styled(workdir, thr_label),
        Span::styled(" \u{00b7} ", muted),
        Span::styled(model_bare, accent),
    ];
    if let Some(effort) = effort {
        spans.push(Span::styled(" \u{00b7} ", muted));
        spans.push(Span::styled(effort, pink));
    }
    Line::from(spans)
}

/// Decide which steer/queue sources the queue panel should show.
///
/// When a *running* subagent is focused we show its child view's steer items;
/// otherwise we fall back to the parent's steer and queue lists.
#[allow(clippy::type_complexity)]
pub(super) fn steer_queue_sources<'a>(
    chat: &'a ChatView,
    subagent_focus: Option<usize>,
    queue_items: &'a [(i64, String)],
) -> (&'a [(i64, String)], &'a [(i64, String)]) {
    // Same liveness rule the `>` click path uses
    // (`subagent_input::is_live_subagent_focus`): what the panel shows and
    // where the click routes must never diverge.
    if let Some(ChatBlock::Subagent { view, .. }) =
        subagent_focus.and_then(|idx| chat.blocks.get(idx))
    {
        if super::subagent_input::is_live_subagent_focus(chat, subagent_focus) {
            return (&view.steer_items, &[][..]);
        }
    }
    (&chat.steer_items, queue_items)
}

/// Input is disabled only when a DONE subagent is focused (not when a running
/// one is — the user can still steer it).
pub(super) fn is_input_disabled(chat: &ChatView, subagent_focus: Option<usize>) -> bool {
    // Done (or stale) focus disables the composer; a live subagent keeps it
    // open so the user can still steer the child.
    subagent_focus.is_some() && !super::subagent_input::is_live_subagent_focus(chat, subagent_focus)
}

/// Timer value shown after the latest body message.
///
/// While an LLM round is streaming (llm_round_started_at_ms is Some) the
/// value counts up live. Between rounds (anchor cleared, frozen_round_ms
/// set) the value holds the frozen final cost of the last round so the timer
/// stays visible during inter-round tool execution. Terminal/idle views return
/// zero.
pub(super) fn display_tail_ms(
    chat: &ChatView,
    subagent_focus: Option<usize>,
    now: i64,
    running: bool,
) -> u64 {
    if let Some(idx) = subagent_focus {
        match chat.blocks.get(idx) {
            Some(ChatBlock::Subagent {
                view, done: false, ..
            }) => round_or_frozen(view, now),
            _ => 0,
        }
    } else if running {
        round_or_frozen(chat, now)
    } else {
        0
    }
}

/// Live round elapsed when streaming, otherwise the frozen cost of the last
/// completed round (zero if none).
fn round_or_frozen(chat: &ChatView, now: i64) -> u64 {
    if let Some(started) = chat.llm_round_started_at_ms {
        ((now - started).max(0)) as u64
    } else {
        chat.frozen_round_ms.unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subagent_block(round_started_at_ms: Option<i64>, done: bool) -> ChatBlock {
        let view = ChatView {
            llm_round_started_at_ms: round_started_at_ms,
            ..Default::default()
        };
        ChatBlock::Subagent {
            id: "sub-1".to_string(),
            child_session_id: "child-1".to_string(),
            kind: "explore".to_string(),
            prompt: "test".to_string(),
            view,
            done,
            ok: false,
            cancelled: false,
            summary: String::new(),
            started_at_ms: 500,
            elapsed_ms: None,
        }
    }

    #[test]
    fn running_subagent_shows_live_elapsed() {
        let mut chat = ChatView::default();
        chat.blocks.push(subagent_block(Some(1000), false));
        // now=5000, started_at_ms=1000 -> 4000
        assert_eq!(display_tail_ms(&chat, Some(0), 5000, true), 4000);
    }

    #[test]
    fn done_subagent_returns_zero() {
        let mut chat = ChatView::default();
        chat.blocks.push(subagent_block(Some(1000), true));
        assert_eq!(display_tail_ms(&chat, Some(0), 5000, true), 0);
    }

    #[test]
    fn top_level_live_round_counts_up() {
        // Active round: llm_round anchor is set, live elapsed.
        let chat = ChatView {
            llm_round_started_at_ms: Some(1000),
            ..Default::default()
        };
        assert_eq!(display_tail_ms(&chat, None, 5000, true), 4000);
        assert_eq!(display_tail_ms(&chat, None, 5000, false), 0);
    }

    #[test]
    fn between_rounds_freezes_last_round_cost() {
        // Between LLM rounds: round anchor cleared, frozen cost carried over
        // so the timer holds the last round final value (not zero, not gone).
        let chat = ChatView {
            llm_round_started_at_ms: None,
            frozen_round_ms: Some(4000),
            ..Default::default()
        };
        assert_eq!(display_tail_ms(&chat, None, 5000, true), 4000);
    }

    #[test]
    fn no_round_and_no_frozen_has_no_timer() {
        // Default view: no anchor, no frozen, zero.
        let chat = ChatView::default();
        assert_eq!(display_tail_ms(&chat, None, 5000, true), 0);
    }

    // ----- compose_top_title layout edge cases (pure, terminal-free) -----

    fn title_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn compose_title_segments_carry_graded_colors() {
        let line = compose_top_title(Path::new("/root/opencoder"), "glm-5.2", Some("high"));
        let t = title_text(&line);
        assert_eq!(t, "/root/opencoder \u{00b7} glm-5.2 \u{00b7} high");
        // workdir bold bright blue (the `thr` label colour, matching the
        // tok-cost corner), separators muted, model accent, thinking level
        // pink (Thinking block header).
        assert_eq!(line.spans[0].style.fg, Some(theme::status_label_color()));
        assert!(line.spans[0]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD));
        assert_eq!(line.spans[1].style.fg, Some(theme::muted()));
        assert_eq!(line.spans[2].style.fg, Some(theme::accent()));
        assert_eq!(line.spans[3].style.fg, Some(theme::muted()));
        assert_eq!(line.spans[4].style.fg, Some(theme::pink()));
    }

    #[test]
    fn compose_title_omits_blank_effort_and_sanitizes_values() {
        let line = compose_top_title(Path::new("/root/op\nencoder"), "glm\n5.2", Some("  "));
        assert_eq!(title_text(&line), "/root/op encoder \u{00b7} glm 5.2");
    }

    /// `TOP_ARROW_W` must match the literal rendered in `render.rs` so the
    /// `compose_top_title` padding reserves exactly the arrow's display width.
    /// The label is `"    ⬆    "` (4 spaces + wide ⬆ + 4 spaces).
    #[test]
    fn top_arrow_width_matches_label() {
        assert_eq!(
            TOP_ARROW_W as usize,
            crate::composer::str_width("    \u{2b06}    ")
        );
    }
}
