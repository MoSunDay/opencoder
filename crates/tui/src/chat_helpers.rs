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
            for k in ["command", "path", "description", "pattern", "prompt"] {
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
                    view.steer_items.clear();
                    *elapsed_ms = Some(((now_ms() - *started_at_ms).max(0)) as u64);
                }
            }
        }
    }
}
