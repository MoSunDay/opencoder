//! Flattening for `ChatBlock::StepGroup` — the three-level tool ladder
//! (group row → step row → calls aggregation row → single call output).
//! Extracted from `chat.rs` for the line gate; `collect_headers`
//! (chat_headers.rs) mirrors this line accounting exactly so hit-rects stay
//! aligned with the live render.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{
    theme, Step, GROUP_ROW_CLOSED_PREFIX, GROUP_ROW_OPEN_PREFIX, SPINNER, STEP_ROW_CLOSED_PREFIX,
    STEP_ROW_OPEN_PREFIX, STEP_THINKING_HEADER,
};

/// Append one `StepGroup` block's lines to `out`. Shape (three-level
/// drill-down): the clickable group row `{▸|❯} N steps` (col 0, accent
/// bold) + a live spinner hint while any call anywhere in the group is
/// still running; while the group is closed that row (plus one trailing
/// blank) is the whole block. While it is open, per step: the step row
/// (indent 2); while the step is open its `💭 Thinking` block (header
/// indent 4, body indent 8) and — when it holds calls — the clickable
/// aggregation row `{▸|❯} N function calls` (indent 4); while the call
/// list is open, each call's header row (indent 6) plus that call's output
/// (indent 6) when individually expanded. One trailing blank line.
pub(crate) fn flatten_step_group(
    out: &mut Vec<Line<'static>>,
    open: bool,
    steps: &[Step],
    anim_tick: u32,
) {
    let n = steps.len();
    // L0 group row: `{▸|❯} N steps` + a live spinner hint while any call
    // anywhere in the group is still running.
    let mut spans = vec![Span::styled(
        format!(
            "{}{n} step{}",
            if open {
                GROUP_ROW_OPEN_PREFIX
            } else {
                GROUP_ROW_CLOSED_PREFIX
            },
            if n == 1 { "" } else { "s" }
        ),
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
    if !open {
        out.push(Line::from(""));
        return;
    }
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
        if step.calls.is_empty() {
            continue;
        }
        // L2 aggregation row: `{▸|❯} N function calls` — one click away
        // from the per-call header rows.
        let m = step.calls.len();
        out.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!(
                    "{}{m} function call{}",
                    if step.calls_open {
                        GROUP_ROW_OPEN_PREFIX
                    } else {
                        GROUP_ROW_CLOSED_PREFIX
                    },
                    if m == 1 { "" } else { "s" }
                ),
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if !step.calls_open {
            continue;
        }
        for c in &step.calls {
            out.extend(super::types::indented(std::slice::from_ref(&c.header), 6));
            // Per-call expansion: only the toggled call shows its output.
            if c.expanded {
                out.extend(super::types::indented(&c.output, 6));
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
