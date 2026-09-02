//! Centralised colour / style theme for the TUI.
//!
//! All semantic colours and reusable block presets live here so that the
//! rendering modules share a single source of truth. There is exactly one
//! fixed palette ([`DARK`], tuned for dark terminal backgrounds): every
//! helper resolves straight to its slots so call sites stay class-free and
//! need no shared mutable handle of their own.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

// ── Semantic colour palette (16-colour base for broad compatibility) ───────

/// Primary accent — interactive highlights, links, focused borders.
pub const ACCENT: Color = Color::Cyan;
/// Caution / warning — plan mode, high context usage.
pub const WARN: Color = Color::Yellow;
/// Success / assistant output.
pub const OK: Color = Color::Green;
/// Error / critical context usage.
pub const ERR: Color = Color::Red;
/// Informational — steer items.
pub const INFO: Color = Color::Blue;
/// Dimmed text — hints, secondary labels, border lines.
pub const MUTED: Color = Color::DarkGray;
/// Subtle text — descriptions, less-important values.
pub const SUBTLE: Color = Color::Gray;
/// Primary text colour.
pub const TEXT: Color = Color::White;
/// Local / non-context information shown to the user that never enters the
/// model context (e.g. `/ps` / `/stop` echoes). Mirrors the `[model]` marker.
pub const LOCAL: Color = Color::Magenta;
/// Pink — dedicated to the Thinking (reasoning) block header so it stays
/// visually distinct from the cyan-accented bash tool header.
pub const PINK: Color = Color::LightMagenta;
/// Dark purple — the Compaction (context-summary) block tag + text, darker
/// than `LOCAL` so the collapsed summary reads as secondary. ANSI 256
/// index 90 is (135,0,135): a dark purple, darker than plain Magenta.
pub const COMPACTION: Color = Color::Indexed(90);

// ── Palette ─────────────────────────────────────────────────────────────────

/// A complete semantic colour set.
pub struct Palette {
    pub accent: Color,
    pub text: Color,
    pub muted: Color,
    pub subtle: Color,
    pub warn: Color,
    pub ok: Color,
    pub err: Color,
    pub info: Color,
    pub local: Color,
    pub pink: Color,
    pub compaction: Color,
    pub user: Color,
}

/// The one and only palette: dark, mirroring the `const` colours above.
pub const DARK: Palette = Palette {
    user: Color::Indexed(220),
    accent: Color::Cyan,
    text: Color::White,
    muted: Color::DarkGray,
    subtle: Color::Gray,
    warn: Color::Yellow,
    ok: Color::Green,
    err: Color::Red,
    info: Color::Blue,
    local: Color::Magenta,
    pink: Color::LightMagenta,
    compaction: Color::Indexed(90),
};

// ── Semantic colours (slots of the fixed [`DARK`] palette) ──────────────────
// Each returns the matching field of the palette; see the `const`
// block above for the meaning of each slot.

pub fn accent() -> Color {
    DARK.accent
}
pub fn text() -> Color {
    DARK.text
}
pub fn muted() -> Color {
    DARK.muted
}
pub fn subtle() -> Color {
    DARK.subtle
}
pub fn warn_color() -> Color {
    DARK.warn
}
pub fn ok_color() -> Color {
    DARK.ok
}

/// Status-bar label colour for the `thr` prefix and `ctx (used/limit)`
/// counts: bold bright blue — ANSI 94 (LightBlue), the same brightness tier
/// as cargo's green 92. Fixed: ratio-to-total context labels take no palette
/// slot, so they never shift with the semantic colours.
pub fn status_label_color() -> Color {
    Color::LightBlue
}
pub fn err_color() -> Color {
    DARK.err
}
pub fn info_color() -> Color {
    DARK.info
}
pub fn local_color() -> Color {
    DARK.local
}
pub fn pink() -> Color {
    DARK.pink
}
pub fn compaction_color() -> Color {
    DARK.compaction
}
pub fn user_color() -> Color {
    DARK.user
}

/// Selection-row background: 256-colour index 238, a dark gray that reads as
/// a highlight on dark terminal backgrounds.
pub fn highlight_bg() -> Color {
    Color::Indexed(238)
}

// ── Block presets ───────────────────────────────────────────────────────────

/// Rounded, plain-bordered block (no title). The base for all panels.
pub fn rounded_block_plain() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted()))
}

/// Rounded block with a title and subtle (muted) border.
pub fn rounded_block(title: &str) -> Block<'static> {
    rounded_block_plain().title(format!(" {} ", title))
}

/// Rounded block with a styled multi-span title (keeps per-segment colors,
/// e.g. the top-title `workdir · [mode] · model` composition) and subtle
/// (muted) border. Padded with a leading/trailing space like [`rounded_block`].
pub fn rounded_block_line(title: &Line<'static>) -> Block<'static> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(title.spans.iter().cloned());
    spans.push(Span::raw(" "));
    rounded_block_plain().title(Line::from(spans))
}

/// [`rounded_block_line`] plus a session-lifetime `[tok cost]` label (and,
/// when `turn_ms > 0`, a `·`-separated `[call cost]` duration) on the bottom
/// border's left corner — the fourth info corner of the body block (right-
/// bottom holds the follow indicator). Both segments render in the warn
/// colour — the same as the status bar's task timer and running-spinner
/// spans — so the accent stays reserved for the live `跟随中…` indicator
/// and the model name, and the `·` separator nearly vanishes (muted).
/// `area_w`
/// guards narrow terminals with graded dropping: when the `[tok cost]`
/// segment alone could collide with the right-bottom indicator (which
/// reserves ~10 display columns at the right edge) the whole label is
/// dropped; when only the `[call cost]` addition would overflow, just the
/// tok segment is kept. `turn_ms == 0` omits the turn segment entirely.
/// Pure builder — no globals.
pub fn rounded_block_line_tok(
    title: &Line<'static>,
    tokens_total: u64,
    area_w: u16,
    turn_ms: u64,
) -> Block<'static> {
    let block = rounded_block_line(title);
    let tok = format!(
        "[tok cost {}]",
        crate::fmt::format_tokens_cost_m(tokens_total)
    );
    let turn = if turn_ms > 0 {
        format!("[call cost {}]", crate::fmt::format_run_duration(turn_ms))
    } else {
        String::new()
    };
    // Right-edge reservation: follow/jump indicator ("    v    " style) plus
    // spacing, never wider than this.
    const RIGHT_RESERVE: u16 = 12;
    let avail = area_w.saturating_sub(2).saturating_sub(RIGHT_RESERVE);
    // `·` (U+00B7) is width-1, so char count == display width.
    let tok_w = tok.chars().count() as u16 + 2; // surrounding spaces
    let turn_w = turn.chars().count() as u16;
    if avail < tok_w {
        return block; // too narrow: drop the whole bottom label
    }
    let show_turn = turn_ms > 0 && avail >= tok_w + turn_w;
    // Graded spans: labels in the warn colour (the status bar's task-timer /
    // running-spinner colour), separator nearly invisible (muted) — accent
    // stays reserved for the live follow indicator. (` · ` adds one column
    // per side vs. the width math above, absorbed by the surrounding-space
    // margin.)
    let label = Style::default().fg(warn_color());
    let mut spans = vec![Span::raw(" "), Span::styled(tok, label)];
    if show_turn {
        spans.push(Span::styled(" \u{00b7} ", Style::default().fg(muted())));
        spans.push(Span::styled(turn, label));
    }
    spans.push(Span::raw(" "));
    block.title_bottom(Line::from(spans).alignment(ratatui::layout::Alignment::Left))
}

/// Pad a title [`Line`]'s spans with one leading/trailing space (mirroring
/// [`rounded_block_line`]) and recolour every span to `fg`. Returns an owned
/// line intended for a *right-aligned* top-border title, e.g. the
/// `/annotation` editor shows the body `workdir · model · effort` title in
/// the annotation accent colour alongside the left ` edit annotation ` label.
pub fn title_spans_colored(line: &Line<'_>, fg: Color) -> Line<'static> {
    let style = Style::default().fg(fg);
    let mut spans = vec![Span::styled(" ", style)];
    for span in &line.spans {
        spans.push(Span::styled(span.content.as_ref().to_string(), style));
    }
    spans.push(Span::styled(" ", style));
    Line::from(spans)
}

/// Rounded block with an accent-coloured border and title.
pub fn rounded_block_focus(title: &str) -> Block<'static> {
    rounded_block_plain()
        .border_style(Style::default().fg(accent()))
        .title(format!(" {} ", title))
}

/// Rounded block with a custom border colour and title.
pub fn rounded_block_color(title: &str, color: Color) -> Block<'static> {
    rounded_block_plain()
        .border_style(Style::default().fg(color))
        .title(format!(" {} ", title))
}

// ── List / status helpers ──────────────────────────────────────────────────

/// Subtle highlight style for the selected list row. The background comes
/// from [`highlight_bg`] (256-colour index 238 on dark, 252 on light).
pub fn list_highlight() -> Style {
    Style::default()
        .bg(highlight_bg())
        .add_modifier(Modifier::BOLD)
}

/// 10-segment visual progress meter for context-window usage. Returns the
/// bar string and a semantic colour based on the percentage.
pub fn context_meter(pct: u8) -> (String, Color) {
    let filled = (pct as usize).min(100) / 10;
    let bar = "\u{25b0}".repeat(filled) + &"\u{25b1}".repeat(10 - filled);
    let color = if pct > 80 {
        err_color()
    } else if pct > 40 {
        warn_color()
    } else {
        ok_color()
    };
    (bar, color)
}

/// Agent chip foreground colour: warning colour in plan (read-only) mode,
/// accent otherwise. `sidecar` gets its own magenta hue so the focused
/// sidecar chip is distinguishable from both act and plan.
pub fn agent_chip_fg(agent: &str) -> Color {
    if agent == "plan" {
        warn_color()
    } else if agent == "sidecar" {
        sidecar_color()
    } else {
        accent()
    }
}

/// Sidecar accent hue (magenta): used by the `⇲ sidecar` block label and the
/// `[sidecar]` mode chip while the sidecar box is focused. Distinct from the
/// plan warn / act accent mapping so the three chips never collide.
pub fn sidecar_color() -> Color {
    Color::Magenta
}

/// Foreground of the parent status-bar mode dot + `[mode]` chip: the plain
/// [`agent_chip_fg`] mapping, except the `act` chip lights up in the sandbox
/// warning hue while the committed skill is `task-plan`. The yellow reverts
/// when any other skill is committed, or a steer/queued input without a
/// `$task-plan` token takes effect; a consumed input that does carry the
/// token re-arms it (the runner newly activates that skill at the
/// consumption boundary, matching an idle submit).
pub fn status_chip_fg(mode: &str, plan_skill_active: bool) -> Color {
    if mode == "act" && plan_skill_active {
        warn_color()
    } else {
        agent_chip_fg(mode)
    }
}

/// Background colour of the plan/act mode-flash chip: warning colour for
/// plan, accent for act — the same mapping as [`agent_chip_fg`], so the
/// chip and the flash can never render different hues. Lives in the theme
/// module (migrated from render.rs) to keep render.rs within its line
/// budget.
pub fn mode_flash_bg(is_plan: bool) -> Color {
    if is_plan {
        warn_color()
    } else {
        accent()
    }
}

// ── Style shortcuts ─────────────────────────────────────────────────────────

/// Bold text in the given colour.
pub fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

/// Dimmed/muted style for secondary text.
pub fn muted_style() -> Style {
    Style::default().fg(muted())
}

/// Subtle style for descriptions.
pub fn subtle_style() -> Style {
    Style::default().fg(subtle())
}

/// Style for local / non-context information that is shown to the user but
/// never sent to the model.
pub fn local_style() -> Style {
    Style::default().fg(local_color())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── palette (pure — const consistency) ───────────────────────────────

    #[test]
    fn status_label_color_is_ansi_bright_blue() {
        // `thr` / `ctx` labels keep the bright (ANSI 9x) tier but in blue:
        // ESC[1m ESC[94m — bold bright blue (ANSI 94 = LightBlue), fixed.
        assert_eq!(status_label_color(), Color::LightBlue);
    }

    #[test]
    fn dark_palette_matches_constants() {
        assert_eq!(DARK.accent, ACCENT);
        assert_eq!(DARK.text, TEXT);
        assert_eq!(DARK.muted, MUTED);
        assert_eq!(DARK.subtle, SUBTLE);
        assert_eq!(DARK.warn, WARN);
        assert_eq!(DARK.ok, OK);
        assert_eq!(DARK.err, ERR);
        assert_eq!(DARK.info, INFO);
        assert_eq!(DARK.local, LOCAL);
        assert_eq!(DARK.pink, PINK);
        assert_eq!(DARK.compaction, COMPACTION);
        // `user` has no const twin: gold (ANSI 220) on the fixed palette.
        assert_eq!(DARK.user, Color::Indexed(220));
        // Semantic helpers must resolve to the same slots.
        assert_eq!(accent(), DARK.accent);
        assert_eq!(user_color(), DARK.user);
        assert_eq!(highlight_bg(), Color::Indexed(238));
    }

    // ── context_meter: behavioural thresholds + bar construction ──────────

    fn assert_meter(pct: u8, filled: usize, color: Color) {
        let (bar, c) = context_meter(pct);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&ch| ch == '\u{25b0}').count(), filled);
        assert_eq!(
            bar.chars().filter(|&ch| ch == '\u{25b1}').count(),
            10 - filled
        );
        assert_eq!(c, color);
    }

    #[test]
    fn context_meter_zero_is_all_empty_and_green() {
        assert_meter(0, 0, OK);
    }

    #[test]
    fn context_meter_at_green_ceiling_is_green() {
        // 40 is the highest percentage that stays Green.
        assert_meter(40, 4, OK);
    }

    #[test]
    fn context_meter_just_above_green_threshold_is_warn() {
        assert_meter(41, 4, WARN);
    }

    #[test]
    fn context_meter_at_warn_ceiling_is_warn() {
        // 80 is the highest percentage that stays Yellow.
        assert_meter(80, 8, WARN);
    }

    #[test]
    fn context_meter_red_threshold_is_red() {
        assert_meter(81, 8, ERR);
    }

    #[test]
    fn context_meter_full_is_all_filled_and_red() {
        assert_meter(100, 10, ERR);
    }

    #[test]
    fn context_meter_clamps_overflow() {
        // Values above 100 clamp to 100 (all filled, Red).
        assert_meter(255, 10, ERR);
    }

    // ── agent_chip_fg ────────────────────────────────────────────────────

    #[test]
    fn agent_chip_fg_plan_is_warn() {
        assert_eq!(agent_chip_fg("plan"), WARN);
    }

    #[test]
    fn agent_chip_fg_non_plan_is_accent() {
        assert_eq!(agent_chip_fg("act"), ACCENT);
        // The interlude `sandbox` spelling must no longer map to the plan hue.
        assert_eq!(agent_chip_fg("sandbox"), ACCENT);
        assert_eq!(agent_chip_fg(""), ACCENT);
    }

    #[test]
    fn agent_chip_fg_sidecar_is_distinct_magenta() {
        // The sidecar chip must be distinguishable from both act and plan,
        // and must not leak into the plan_skill_active warn branch of
        // `status_chip_fg` (that branch is act-only).
        let sidecar = agent_chip_fg("sidecar");
        assert_eq!(sidecar, sidecar_color());
        assert_ne!(sidecar, agent_chip_fg("act"));
        assert_ne!(sidecar, agent_chip_fg("plan"));
        assert_eq!(status_chip_fg("sidecar", true), sidecar);
    }

    // -- status_chip_fg ----------------------------------------------------───────────

    #[test]
    fn status_chip_fg_act_lights_yellow_for_task_plan() {
        assert_eq!(status_chip_fg("act", true), WARN);
        assert_eq!(status_chip_fg("act", false), ACCENT);
        // Only the act status changes hue; plan is already the warning colour.
        assert_eq!(status_chip_fg("plan", true), WARN);
        assert_eq!(status_chip_fg("plan", false), WARN);
        assert_eq!(status_chip_fg("explore", true), ACCENT);
    }

    // ── user_color ────────────────────────────────────────────────────────

    #[test]
    fn user_color_is_gold() {
        assert_eq!(user_color(), Color::Indexed(220));
    }

    // ── list_highlight ───────────────────────────────────────────────────

    #[test]
    fn list_highlight_has_indexed_bg_and_bold() {
        let s = list_highlight();
        assert_eq!(s.bg, Some(Color::Indexed(238)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    // ── style shortcuts ──────────────────────────────────────────────────

    #[test]
    fn bold_sets_fg_and_bold_modifier() {
        let s = bold(WARN);
        assert_eq!(s.fg, Some(WARN));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn muted_style_is_muted() {
        assert_eq!(muted_style().fg, Some(MUTED));
    }

    #[test]
    fn subtle_style_is_subtle() {
        assert_eq!(subtle_style().fg, Some(SUBTLE));
    }

    #[test]
    fn local_style_is_local() {
        assert_eq!(local_style().fg, Some(LOCAL));
    }

    // ── rounded_block_line ──────────────────────────────────────────────

    #[test]
    fn rounded_block_line_pads_title_like_rounded_block() {
        // multi-span styled title (per-segment colors kept by the Line)
        let line = Line::from(vec![
            Span::styled("workdir", Style::default().fg(Color::Cyan)),
            Span::raw(" \u{00b7} [act]"),
        ]);
        let from_line = rounded_block_line(&line);
        let from_str = rounded_block("workdir \u{00b7} [act]");

        let top_row = |block: Block<'static>| {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 3)).unwrap();
            terminal.draw(|f| f.render_widget(block, f.area())).unwrap();
            let buf = terminal.backend().buffer();
            let mut s = String::new();
            for x in 0..40 {
                if let Some(cell) = buf.cell((x, 0)) {
                    s.push_str(cell.symbol());
                }
            }
            s
        };

        let line_row = top_row(from_line);
        let str_row = top_row(from_str);
        assert_eq!(
            line_row, str_row,
            "rounded_block_line must render the identical title row (same \
             leading/trailing space padding) as rounded_block"
        );
        assert!(
            line_row.contains(" workdir \u{00b7} [act] "),
            "title must carry one padding space on each side; got: {line_row}"
        );
    }

    // ── title_spans_colored ───────────────────────────────────────────────

    #[test]
    fn title_spans_colored_pads_and_recolors_all_spans() {
        let green = ok_color();
        let line = Line::from(vec![
            Span::raw("workdir"),
            Span::raw(" \u{00b7} "),
            Span::raw("glm-5.2"),
        ]);
        let out = title_spans_colored(&line, green);

        // 3 content spans + 1 leading + 1 trailing padding span.
        assert_eq!(out.spans.len(), 5, "expected 3 content + 2 padding spans");
        assert_eq!(out.spans.first().unwrap().content.as_ref(), " ");
        assert_eq!(out.spans.last().unwrap().content.as_ref(), " ");

        // Content preserved in order with separators and outer padding.
        let joined: String = out.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, " workdir \u{00b7} glm-5.2 ");

        // Every span recoloured to `fg`, overriding the original raw spans.
        for span in &out.spans {
            assert_eq!(span.style.fg, Some(green), "span not green: {joined}");
        }
    }
}
