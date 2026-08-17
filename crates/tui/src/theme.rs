//! Centralised colour / style theme for the TUI.
//!
//! All semantic colours and reusable block presets live here so that the
//! rendering modules share a single source of truth. A single piece of
//! global state selects between a `dark` (default) and `light` palette at
//! runtime; every helper resolves through [`current_theme`] so call sites
//! stay class-free and need no shared mutable handle of their own.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use std::sync::{OnceLock, RwLock};

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

// ── Theme selection ─────────────────────────────────────────────────────────

/// The two supported colour themes. `Dark` is the default and matches the
/// `const` palette above; `Light` is tuned for white-background terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Dark,
    Light,
}

impl ThemeKind {
    /// Stable lowercase label, matching the `theme` config string.
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "dark",
            ThemeKind::Light => "light",
        }
    }

    /// Parse a label: `trim` + `to_lowercase` first; only `"light"` yields
    /// [`ThemeKind::Light`], anything else falls back to [`ThemeKind::Dark`].
    pub fn from_label(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "light" => ThemeKind::Light,
            _ => ThemeKind::Dark,
        }
    }

    /// Toggle between the two themes.
    pub fn next(self) -> Self {
        match self {
            ThemeKind::Dark => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::Dark,
        }
    }
}

/// A complete semantic colour set for one theme.
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

/// Pure lookup: the palette for the given theme. Dark mirrors the `const`
/// palette; light swaps contrast for white-background terminals.
pub fn palette(kind: ThemeKind) -> Palette {
    match kind {
        ThemeKind::Dark => Palette {
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
        },
        ThemeKind::Light => Palette {
            user: Color::Indexed(94),
            accent: Color::Blue,
            text: Color::Black,
            muted: Color::Gray,
            subtle: Color::DarkGray,
            warn: Color::LightRed,
            ok: Color::Green,
            err: Color::Red,
            info: Color::Blue,
            local: Color::Magenta,
            pink: Color::Magenta,
            compaction: Color::Indexed(90),
        },
    }
}

static THEME: OnceLock<RwLock<ThemeKind>> = OnceLock::new();

/// Set the active theme globally.
pub fn set_theme(kind: ThemeKind) {
    let lock = THEME.get_or_init(|| RwLock::new(ThemeKind::Dark));
    if let Ok(mut guard) = lock.write() {
        *guard = kind;
    }
}

/// The active theme. Defaults to [`ThemeKind::Dark`] until [`set_theme`] runs.
pub fn current_theme() -> ThemeKind {
    THEME
        .get_or_init(|| RwLock::new(ThemeKind::Dark))
        .read()
        .map(|g| *g)
        .unwrap_or(ThemeKind::Dark)
}

// ── Semantic colours (resolved via `current_theme`) ─────────────────────────
// Each returns the matching field of the active palette; see the `const`
// block above for the meaning of each slot (the dark values).

pub fn accent() -> Color {
    palette(current_theme()).accent
}
pub fn text() -> Color {
    palette(current_theme()).text
}
pub fn muted() -> Color {
    palette(current_theme()).muted
}
pub fn subtle() -> Color {
    palette(current_theme()).subtle
}
pub fn warn_color() -> Color {
    palette(current_theme()).warn
}
pub fn ok_color() -> Color {
    palette(current_theme()).ok
}

/// Status-bar label colour for the `thr` prefix and `ctx (used/limit)`
/// counts: bold bright blue — ANSI 94 (LightBlue), the same brightness tier
/// as cargo's green 92. Theme-independent: ratio-to-total context labels do
/// not change with the palette.
pub fn status_label_color() -> Color {
    Color::LightBlue
}
pub fn err_color() -> Color {
    palette(current_theme()).err
}
pub fn info_color() -> Color {
    palette(current_theme()).info
}
pub fn local_color() -> Color {
    palette(current_theme()).local
}
pub fn pink() -> Color {
    palette(current_theme()).pink
}
pub fn compaction_color() -> Color {
    palette(current_theme()).compaction
}
pub fn user_color() -> Color {
    palette(current_theme()).user
}

/// Selection-row background. Dark uses 256-colour index 238, light uses the
/// softer 252 for white-background terminals.
pub fn highlight_bg() -> Color {
    match current_theme() {
        ThemeKind::Dark => Color::Indexed(238),
        ThemeKind::Light => Color::Indexed(252),
    }
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

/// Agent chip foreground colour: warning colour in plan mode, accent otherwise.
pub fn agent_chip_fg(agent: &str) -> Color {
    if agent == "plan" {
        warn_color()
    } else {
        accent()
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

    // ── ThemeKind / palette (pure — no global-state mutation) ────────────

    #[test]
    fn status_label_color_is_ansi_bright_blue() {
        // `thr` / `ctx` labels keep the bright (ANSI 9x) tier but in blue:
        // ESC[1m ESC[94m — bold bright blue (ANSI 94 = LightBlue), fixed
        // regardless of the active palette.
        assert_eq!(status_label_color(), Color::LightBlue);
        set_theme(ThemeKind::Light);
        assert_eq!(status_label_color(), Color::LightBlue);
        set_theme(ThemeKind::Dark);
    }

    #[test]
    fn theme_kind_label_roundtrip() {
        assert_eq!(
            ThemeKind::from_label(ThemeKind::Dark.label()),
            ThemeKind::Dark
        );
        assert_eq!(
            ThemeKind::from_label(ThemeKind::Light.label()),
            ThemeKind::Light
        );
        assert_eq!(ThemeKind::from_label("light"), ThemeKind::Light);
        assert_eq!(ThemeKind::from_label("Light"), ThemeKind::Light);
        assert_eq!(ThemeKind::from_label(" LIGHT "), ThemeKind::Light);
        assert_eq!(ThemeKind::from_label("foo"), ThemeKind::Dark);
    }

    #[test]
    fn theme_kind_next() {
        assert_eq!(ThemeKind::Dark.next(), ThemeKind::Light);
        assert_eq!(ThemeKind::Light.next(), ThemeKind::Dark);
    }

    #[test]
    fn palette_dark_matches_constants() {
        let p = palette(ThemeKind::Dark);
        assert_eq!(p.accent, ACCENT);
        assert_eq!(p.text, TEXT);
        assert_eq!(p.muted, MUTED);
        assert_eq!(p.subtle, SUBTLE);
        assert_eq!(p.warn, WARN);
        assert_eq!(p.ok, OK);
        assert_eq!(p.err, ERR);
        assert_eq!(p.info, INFO);
        assert_eq!(p.local, LOCAL);
    }

    #[test]
    fn palette_light_text_is_black() {
        let p = palette(ThemeKind::Light);
        assert_eq!(p.text, Color::Black);
        assert_eq!(p.accent, Color::Blue);
        assert_eq!(p.warn, Color::LightRed);
    }

    #[test]
    fn set_then_current_theme() {
        // `THEME` is a process-wide global shared by many parallel tests
        // across the crate (render_tests, chat_tests, …), all of which call
        // `set_theme`. Setting Light and reading it back is therefore racy:
        // a concurrent `set_theme(Dark)` can land between the two calls.
        //
        // Hold an exclusive write lock for the whole critical section and
        // exercise the storage directly. Because `set_theme`/`current_theme`
        // take this same lock, they block until we restore the default —
        // making this deterministic with no re-entrant deadlock.
        let lock = THEME.get_or_init(|| RwLock::new(ThemeKind::Dark));
        let mut guard = lock.write().unwrap();
        *guard = ThemeKind::Light;
        assert_eq!(*guard, ThemeKind::Light);
        // restore default so other tests see dark
        *guard = ThemeKind::Dark;
    }

    // ── context_meter: behavioural thresholds + bar construction ──────────

    fn assert_meter(pct: u8, filled: usize, color: Color) {
        set_theme(ThemeKind::Dark);
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
        set_theme(ThemeKind::Dark);
        assert_eq!(agent_chip_fg("plan"), WARN);
    }

    #[test]
    fn agent_chip_fg_non_plan_is_accent() {
        set_theme(ThemeKind::Dark);
        assert_eq!(agent_chip_fg("act"), ACCENT);
        assert_eq!(agent_chip_fg(""), ACCENT);
    }

    // ── user_color ────────────────────────────────────────────────────────

    #[test]
    fn user_color_is_gold_in_dark_theme() {
        assert_eq!(palette(ThemeKind::Dark).user, Color::Indexed(220));
    }

    #[test]
    fn user_color_is_dark_gold_in_light_theme() {
        assert_eq!(palette(ThemeKind::Light).user, Color::Indexed(94));
    }

    // ── list_highlight ───────────────────────────────────────────────────

    #[test]
    fn list_highlight_has_indexed_bg_and_bold() {
        set_theme(ThemeKind::Dark);
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
        set_theme(ThemeKind::Dark);
        assert_eq!(muted_style().fg, Some(MUTED));
    }

    #[test]
    fn subtle_style_is_subtle() {
        set_theme(ThemeKind::Dark);
        assert_eq!(subtle_style().fg, Some(SUBTLE));
    }

    #[test]
    fn local_style_is_local() {
        set_theme(ThemeKind::Dark);
        assert_eq!(local_style().fg, Some(LOCAL));
    }

    // ── rounded_block_line ──────────────────────────────────────────────

    #[test]
    fn rounded_block_line_pads_title_like_rounded_block() {
        set_theme(ThemeKind::Dark);
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
        set_theme(ThemeKind::Dark);
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
