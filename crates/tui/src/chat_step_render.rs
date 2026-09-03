//! Flattening for `ChatBlock::StepGroup` — the three-level tool ladder
//! (turn row → step content/calls aggregate → function-call result).
//! Extracted from `chat.rs` for the line gate; `collect_headers`
//! (chat_headers.rs) mirrors this line accounting exactly so hit-rects stay
//! aligned with the live render.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{
    theme, Step, ToolCall, GROUP_ROW_CLOSED_PREFIX, GROUP_ROW_OPEN_PREFIX, SPINNER,
    STEP_ROW_CLOSED_PREFIX, STEP_ROW_OPEN_PREFIX, STEP_THINKING_HEADER,
};

/// Append one `StepGroup` block's lines to `out`. Shape (three-level
/// drill-down): the clickable turn row `{▸|❯} N Steps` (col 0, accent
/// bold) + a live progress hint until Say begins; while the group is closed
/// that row (plus one trailing
/// blank) is the whole block. While it is open, per step: the step row
/// (indent 2); while the step is open its `💭 Thinking` block (header
/// indent 4, body indent 8) and a `N Function calls` aggregation row
/// (indent 4); opening the aggregation shows call headers at indent 6, and
/// an expanded call shows its result at indent 6. One trailing blank line.
pub(crate) fn flatten_step_group(
    out: &mut Vec<Line<'static>>,
    open: bool,
    progress_active: bool,
    steps: &[Step],
    anim_tick: u32,
) {
    let n = steps.len();
    // L0 group row: `{▸|❯} N Steps` + a live spinner hint from step/tool
    // activity until the next Say starts. The two leading spaces keep motion
    // visually separate from the count without adding a row.
    let mut spans = vec![Span::styled(
        format!(
            "{}{n} Step{}",
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
    if progress_active {
        spans.push(Span::styled(
            format!(
                "  {} running ",
                SPINNER[(anim_tick as usize) % SPINNER.len()]
            ),
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
        let m = step.calls.len();
        out.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                format!(
                    "{}{m} Function call{}",
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
            let header = call_header(c);
            out.extend(super::types::indented(std::slice::from_ref(&header), 6));
            // Per-call expansion: only the toggled call shows its output.
            if c.expanded {
                out.extend(super::types::indented(&c.output, 6));
                out.push(Line::from(""));
            }
        }
    }
    out.push(Line::from(""));
}

/// Derive the disclosure glyph without mutating the stored call header.
fn call_header(call: &ToolCall) -> Line<'static> {
    let mut header = call.header.clone();
    let Some(first) = header.spans.first_mut() else {
        return header;
    };
    let text = first.content.to_string();
    let prefix = if call.expanded {
        GROUP_ROW_OPEN_PREFIX
    } else {
        GROUP_ROW_CLOSED_PREFIX
    };
    if let Some(body) = text
        .strip_prefix(GROUP_ROW_OPEN_PREFIX)
        .or_else(|| text.strip_prefix(GROUP_ROW_CLOSED_PREFIX))
    {
        first.content = format!("{prefix}{body}").into();
    }
    header
}
