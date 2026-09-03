//! Flattening for `ChatBlock::StepGroup` — the two-level tool ladder
//! (step → single call output) under a static `≡ N steps` marker.
//! Extracted from `chat.rs` for the line gate; `collect_headers`
//! (chat_headers.rs) mirrors this line accounting exactly so hit-rects stay
//! aligned with the live render.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{
    theme, Step, SPINNER, STEP_ROW_CLOSED_PREFIX, STEP_ROW_OPEN_PREFIX, STEP_THINKING_HEADER,
};

/// Append one `StepGroup` block's lines to `out`. Shape: a static marker row
/// `≡ N steps` (col 0, never clickable/collapsible) + a live spinner hint
/// while any call anywhere in the group is still running, then per step:
/// the step row (indent 2) always renders; while the step is open, its
/// `💭 Thinking` block (header indent 4, body indent 8) and each call's
/// header row (indent 4) plus that call's output (indent 4) when
/// individually expanded. One trailing blank line after the rows.
pub(crate) fn flatten_step_group(out: &mut Vec<Line<'static>>, steps: &[Step], anim_tick: u32) {
    let n = steps.len();
    // Static marker row: `≡ N steps` + a live spinner hint while any call
    // anywhere in the group is still running.
    let mut spans = vec![Span::styled(
        format!("\u{2261} {n} step{}", if n == 1 { "" } else { "s" }),
        Style::default()
            .fg(theme::accent())
            .add_modifier(Modifier::BOLD),
    )];
    if running(steps) {
        spans.push(Span::styled(
            format!("{} running ", SPINNER[(anim_tick as usize) % SPINNER.len()]),
            Style::default().fg(theme::warn_color()),
        ));
    }
    out.push(Line::from(spans));
    for (si, step) in steps.iter().enumerate() {
        let step_open = step.open;
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(
                    "{}{})",
                    if step_open {
                        STEP_ROW_OPEN_PREFIX
                    } else {
                        STEP_ROW_CLOSED_PREFIX
                    },
                    si + 1,
                ),
                Style::default()
                    .fg(theme::ok_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if !step_open {
            continue;
        }
        if !step.thinking.is_empty() {
            out.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    STEP_THINKING_HEADER,
                    Style::default()
                        .fg(theme::pink())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            out.extend(super::types::indented(&step.thinking, 8));
        }
        for c in &step.calls {
            out.extend(super::types::indented(std::slice::from_ref(&c.header), 4));
            // Per-call expansion: only the toggled call shows its output.
            if c.expanded {
                out.extend(super::types::indented(&c.output, 4));
                out.push(Line::from(""));
            }
        }
    }
    out.push(Line::from(""));
}

/// Whether any call in the group is still running (spinner hint).
fn running(steps: &[Step]) -> bool {
    steps
        .iter()
        .any(|s| s.calls.iter().any(|c| c.elapsed_ms.is_none()))
}
