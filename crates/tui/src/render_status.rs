//! Status bar rendering extracted from `render.rs` to respect the per-file
//! line cap. Draws the mode chip, context meter, task timer, and spinner.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::fmt as fmtmod;
use crate::theme;

/// Baseline subtracted from used/window for the `thr` percentage. Now that
/// tool-schema tokens are counted in `used` (via `sys_tokens_for`), keeping
/// this at 0 means the meter shows the true percentage of budget consumed.
pub(crate) const CONTEXT_BASELINE: u64 = 0;

/// Braille spinner frames shown while a task is running.
pub(crate) const SPINNER: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

/// Number of 100ms animation ticks in each visible/hidden status-dot phase.
/// A 500ms phase is noticeable without the urgent flicker of a 100ms phase.
const STATUS_DOT_PHASE_TICKS: u32 = 5;

/// Span for the leading status dot that precedes the mode chip. While running
/// the dot alternates every 500ms. Hidden phases use two spaces so the
/// `[act]`/`[plan]` chip never shifts horizontally. Idle (non-running) keeps
/// the dot steady in the mode colour.
fn status_dot(running: bool, anim_tick: u32, mode: &str) -> Span<'static> {
    let hidden_phase = (anim_tick / STATUS_DOT_PHASE_TICKS) % 2 == 1;
    if running && hidden_phase {
        Span::raw("  ")
    } else {
        Span::styled("\u{25cf} ", Style::default().fg(theme::agent_chip_fg(mode)))
    }
}

/// Resolve the context size shown in `ctx (used/limit)`: the provider-truth
/// context of the latest completed round (`input+output`) wins over the local
/// estimate. `sys_tokens` is skipped in real mode — the provider's
/// `input_tokens` already include the system prompt, so adding it would
/// double-count.
pub(crate) fn resolve_ctx_used(real: Option<u64>, context_used: u64, sys_tokens: u64) -> u64 {
    real.unwrap_or(context_used + sys_tokens)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_status(
    f: &mut Frame,
    area: Rect,
    mode: &str,
    running: bool,
    status: &str,
    anim_tick: u32,
    used: u64,
    compaction_threshold: u64,
    context_limit: u64,
    task_ms: u64,
) {
    let mut spans = vec![
        Span::raw(" "),
        status_dot(running, anim_tick, mode),
        Span::styled(
            format!("[{mode}]"),
            Style::default().fg(theme::agent_chip_fg(mode)),
        ),
    ];

    let bar_pct = fmtmod::context_percent(used, compaction_threshold, CONTEXT_BASELINE);
    let (meter, ctx_color) = theme::context_meter(bar_pct);
    spans.push(Span::raw(" \u{00b7} "));
    // Only the meter bar + percent value follow the semantic threshold colour.
    // The `thr` label and the `ctx (used/limit)` counts are ratio-to-total
    // context and use the bold bright-blue status label colour regardless of
    // how full the threshold meter is.
    spans.push(Span::styled(
        "thr ",
        Style::default()
            .fg(theme::status_label_color())
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!("{meter} {bar_pct}% "),
        Style::default().fg(ctx_color),
    ));
    spans.push(Span::styled(
        format!(
            "ctx ({}/{})",
            fmtmod::format_tokens_compact(used),
            fmtmod::format_tokens_compact(context_limit)
        ),
        Style::default()
            .fg(theme::status_label_color())
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw("  "));

    if task_ms > 0 {
        spans.push(Span::styled(
            fmtmod::format_run_duration(task_ms),
            Style::default().fg(theme::warn_color()),
        ));
        spans.push(Span::raw("  "));
    }

    if running {
        let spin = SPINNER[(anim_tick as usize) % SPINNER.len()];
        spans.push(Span::styled(
            format!("{spin} {status}"),
            Style::default().fg(theme::warn_color()),
        ));
    } else if !status.is_empty() {
        spans.push(Span::styled(
            format!("\u{00b7} {status}"),
            Style::default().fg(theme::muted()),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
