//! Flattening for `ChatBlock::StepGroup` — the three-level tool ladder
//! (group → step → single call output). Extracted from `chat.rs` for the
//! line gate; `collect_headers` (chat_headers.rs) mirrors this line
//! accounting exactly so hit-rects stay aligned with the live render.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{theme, Step, ROLE_SAY_HEADER, SPINNER, STEP_ROW_CLOSED_PREFIX, STEP_ROW_OPEN_PREFIX};

/// Append one `StepGroup` block's lines to `out`. Ladder:
/// group row (col 0) → per step: step row (indent 2) →, while the step is
/// open, its `❯ Say:` thinking block (header indent 4, body indent 8) and
/// each call's header row (indent 4) plus that call's output (indent 4) when
/// individually expanded. Trailing blank only while the group is open, so a
/// collapsed group costs exactly one line.
pub(crate) fn flatten_step_group(
    out: &mut Vec<Line<'static>>,
    steps: &[Step],
    open: bool,
    anim_tick: u32,
) {
    let n = steps.len();
    // Group row: `▸ N steps` (arrow flips to ▾ once expanded) + a live
    // spinner hint while any call anywhere in the group is still running.
    let arrow = if open { "\u{25be}" } else { "\u{25b8}" };
    let mut spans = vec![Span::styled(
        format!("{arrow} {n} step{}", if n == 1 { "" } else { "s" }),
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
                    ROLE_SAY_HEADER,
                    Style::default()
                        .fg(theme::ok_color())
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
