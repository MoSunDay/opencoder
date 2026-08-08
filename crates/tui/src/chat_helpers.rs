use crate::chat::ChatView;
use crate::composer;
use crate::terminal_text::sanitize_single_line;

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
