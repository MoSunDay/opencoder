use crate::chat::ChatView;
use crate::composer;
use crate::terminal_text::sanitize_single_line;

use opencoder_core::message::now_ms;

pub(crate) fn summarize(input: &serde_json::Value) -> String {
    // The full value is returned verbatim (trimmed); the transcript body
    // renders with `Paragraph::wrap(Wrap { trim: false })`, so the terminal
    // wraps long commands to its actual width. Never truncate here — an
    // 80-column cut hid the real command behind an ellipsis.
    match input {
        serde_json::Value::Object(m) => {
            for k in [
                "command",
                "path",
                "description",
                "pattern",
                "prompt",
                "question",
            ] {
                if let Some(s) = m.get(k).and_then(|v| v.as_str()) {
                    return sanitize_single_line(s.trim()).into_owned();
                }
            }
            sanitize_single_line(serde_json::to_string(input).unwrap_or_default().trim())
                .into_owned()
        }
        o => sanitize_single_line(serde_json::to_string(o).unwrap_or_default().trim()).into_owned(),
    }
}

/// Truncate `s` to at most `n` *display columns* (not characters), appending
/// an ellipsis when trimmed. Uses composer's width-aware truncation so CJK /
/// emoji text no longer overflows its visual budget.
pub(crate) fn short(s: &str, n: usize) -> String {
    composer::truncate_to_width(&sanitize_single_line(s.trim()), n)
}

/// Read the concatenated text content of all blocks (for testing).
pub fn block_text(view: &ChatView) -> String {
    view.flatten()
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.clone())
        .collect()
}

/// Append a styled duration span to the header. Running → live warn-color
/// timer; done → frozen muted timer (hidden when < 1s to avoid `0s` noise).
/// NOTE: now used only by Subagent headers — the per-call Tool inline timers
/// were removed; the bottom-border corner shows the whole-turn `[call cost]` timer instead.
pub(crate) fn push_duration_span(
    spans: &mut Vec<ratatui::text::Span<'static>>,
    started_at_ms: i64,
    elapsed_ms: Option<u64>,
    now_ms: i64,
) {
    use ratatui::style::Style;
    use ratatui::text::Span;
    let (dur_ms, color) = match elapsed_ms {
        Some(e) if e >= 1000 => (e, crate::theme::muted()),
        Some(_) => return,
        None => {
            let live = ((now_ms - started_at_ms).max(0)) as u64;
            (live, crate::theme::warn_color())
        }
    };
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        crate::fmt::format_run_duration(dur_ms),
        Style::default().fg(color),
    ));
}

/// Reconcile any subagent block left spinning because its `SubagentEnd` was
/// dropped under UI-channel saturation (`forward_event` uses lossy `try_send`
/// for non-delta lifecycle events). Marks such blocks interrupted so no
/// phantom "running" task outlives the turn - which would otherwise defeat
/// the mode-switch running-gate's contract (the gate consults `running` /
/// `subagents_running`, both cleared on `Done`, so an orphaned spinner would
/// let a Shift+Tab mode switch slip through with no interception). Mirrors the
/// resume/replay mapping of a stale `Running` DB row -> "(interrupted)".
impl ChatView {
    pub(crate) fn reconcile_orphaned_subagents(&mut self) {
        for b in &mut self.blocks {
            if let crate::chat::ChatBlock::Subagent {
                done,
                ok,
                cancelled,
                summary,
                view,
                started_at_ms,
                elapsed_ms,
                ..
            } = b
            {
                if !*done {
                    *done = true;
                    *ok = false;
                    *cancelled = false;
                    if summary.is_empty() {
                        *summary = "(interrupted)".to_string();
                    }
                    view.llm_round_started_at_ms = None;
                    view.frozen_round_ms = None;
                    view.steer_items.clear();
                    *elapsed_ms = Some(((now_ms() - *started_at_ms).max(0)) as u64);
                }
            }
        }
    }
}

/// Add bash-command helper methods to [`ChatView`].
impl ChatView {
    /// Push a placeholder single-call `ChatBlock::ToolGroup` for a `!cmd`
    /// execution. The group starts in the Results state so the user sees the
    /// command running with its output. Call [`finish_bash_tool`] to fill in
    /// the output and collapse the group.
    pub(crate) fn push_bash_tool(&mut self, cmd: &str) {
        use crate::theme;
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        self.finalize_assistant();
        self.blocks.push(crate::chat::ChatBlock::ToolGroup {
            calls: vec![crate::chat::ToolCall {
                id: format!("bash-{}", now_ms()),
                header: Line::from(Span::styled(
                    format!("\u{25b8} {}", sanitize_single_line(cmd)),
                    Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                )),
                output: Vec::new(),
                started_at_ms: Some(now_ms()),
                elapsed_ms: None,
                expanded: false,
            }],
            state: crate::chat::ToolGroupState::Results,
        });
    }

    /// Fill the output of the most recent unfinished `bash-` tool call,
    /// collapse its group, and record elapsed time.
    pub(crate) fn finish_bash_tool(&mut self, output: &str) {
        use crate::chat::TOOL_OUTPUT_LINES;
        use crate::terminal_text::sanitize_multiline;
        use crate::theme;
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        let ts = now_ms();
        let clean = sanitize_multiline(output);
        let out: Vec<Line<'static>> = clean
            .lines()
            .take(TOOL_OUTPUT_LINES)
            .map(|l| {
                Line::from(Span::styled(
                    format!("  {l}"),
                    Style::default().fg(theme::muted()),
                ))
            })
            .collect();
        // Newest group holding an unfinished `bash-` call.
        let target = self.blocks.iter().enumerate().rev().find_map(|(gi, blk)| {
            if let crate::chat::ChatBlock::ToolGroup { calls, .. } = blk {
                calls
                    .iter()
                    .position(|c| c.id.starts_with("bash-") && c.elapsed_ms.is_none())
                    .map(|ci| (gi, ci))
            } else {
                None
            }
        });
        if let Some((gi, ci)) = target {
            if let crate::chat::ChatBlock::ToolGroup { calls, state } = &mut self.blocks[gi] {
                let c = &mut calls[ci];
                c.output = out;
                if let Some(started) = c.started_at_ms {
                    c.elapsed_ms = Some(((ts - started).max(0)) as u64);
                }
                // Equivalent of the old "collapse once finished": back to the
                // single group line.
                *state = crate::chat::ToolGroupState::Collapsed;
            }
        }
    }
}
