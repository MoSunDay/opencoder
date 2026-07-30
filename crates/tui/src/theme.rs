//! Centralised colour / style theme for the TUI.
//!
//! All semantic colours and reusable block presets live here so that the
//! rendering modules share a single source of truth. Everything is a free
//! function or constant — no state, no classes.

use ratatui::style::{Color, Modifier, Style};
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

// ── Block presets ───────────────────────────────────────────────────────────

/// Rounded, plain-bordered block (no title). The base for all panels.
pub fn rounded_block_plain() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
}

/// Rounded block with a title and subtle (muted) border.
pub fn rounded_block(title: &str) -> Block<'static> {
    rounded_block_plain().title(format!(" {} ", title))
}

/// Rounded block with an accent-coloured border and title.
pub fn rounded_block_focus(title: &str) -> Block<'static> {
    rounded_block_plain()
        .border_style(Style::default().fg(ACCENT))
        .title(format!(" {} ", title))
}

/// Rounded block with a custom border colour and title.
pub fn rounded_block_color(title: &str, color: Color) -> Block<'static> {
    rounded_block_plain()
        .border_style(Style::default().fg(color))
        .title(format!(" {} ", title))
}

// ── List / status helpers ──────────────────────────────────────────────────

/// Subtle highlight style for the selected list row. Uses 256-colour index 238
/// for a softer background than plain `DarkGray`.
pub fn list_highlight() -> Style {
    Style::default()
        .bg(Color::Indexed(238))
        .add_modifier(Modifier::BOLD)
}

/// 10-segment visual progress meter for context-window usage. Returns the
/// bar string and a semantic colour based on the percentage.
pub fn context_meter(pct: u8) -> (String, Color) {
    let filled = (pct as usize).min(100) / 10;
    let bar = "\u{25b0}".repeat(filled) + &"\u{25b1}".repeat(10 - filled);
    let color = if pct >= 85 {
        ERR
    } else if pct >= 60 {
        WARN
    } else {
        OK
    };
    (bar, color)
}

/// Agent chip foreground colour: Yellow in plan mode, Cyan otherwise.
pub fn agent_chip_fg(agent: &str) -> Color {
    if agent == "plan" {
        WARN
    } else {
        ACCENT
    }
}

// ── Style shortcuts ─────────────────────────────────────────────────────────

/// Bold text in the given colour.
pub fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

/// Dimmed/muted style for secondary text.
pub fn muted_style() -> Style {
    Style::default().fg(MUTED)
}

/// Subtle style for descriptions.
pub fn subtle_style() -> Style {
    Style::default().fg(SUBTLE)
}

/// Style for local / non-context information that is shown to the user but
/// never sent to the model.
pub fn local_style() -> Style {
    Style::default().fg(LOCAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── context_meter: behavioural thresholds + bar construction ──────────

    #[test]
    fn context_meter_zero_is_all_empty_and_green() {
        let (bar, color) = context_meter(0);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 0);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b1}').count(), 10);
        assert_eq!(color, OK);
    }

    #[test]
    fn context_meter_just_below_yellow_threshold_is_green() {
        // 59 is the highest percentage that stays Green.
        let (bar, color) = context_meter(59);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 5);
        assert_eq!(color, OK);
    }

    #[test]
    fn context_meter_yellow_threshold_is_yellow() {
        let (bar, color) = context_meter(60);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 6);
        assert_eq!(color, WARN);
    }

    #[test]
    fn context_meter_just_below_red_threshold_is_yellow() {
        // 84 is the highest percentage that stays Yellow.
        let (bar, color) = context_meter(84);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 8);
        assert_eq!(color, WARN);
    }

    #[test]
    fn context_meter_red_threshold_is_red() {
        let (bar, color) = context_meter(85);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 8);
        assert_eq!(color, ERR);
    }

    #[test]
    fn context_meter_full_is_all_filled_and_red() {
        let (bar, color) = context_meter(100);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b1}').count(), 0);
        assert_eq!(color, ERR);
    }

    #[test]
    fn context_meter_clamps_overflow() {
        // Values above 100 clamp to 100 (all filled, Red).
        let (bar, color) = context_meter(255);
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '\u{25b0}').count(), 10);
        assert_eq!(color, ERR);
    }

    // ── agent_chip_fg ────────────────────────────────────────────────────

    #[test]
    fn agent_chip_fg_plan_is_warn() {
        assert_eq!(agent_chip_fg("plan"), WARN);
    }

    #[test]
    fn agent_chip_fg_non_plan_is_accent() {
        assert_eq!(agent_chip_fg("act"), ACCENT);
        assert_eq!(agent_chip_fg(""), ACCENT);
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
}
