//! Rendering for the `/ap` mode picker (mirrors `skill_menu/view.rs`):
//! centered popup anchored just above the composer, cursor row in bold,
//! the active mode kept in accent color with a `← 当前` mark.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::list::AP_CHOICES;
use super::state::ApMenu;

/// Width reserved for the mode-key column (padded).
const KEY_W: usize = 8;

/// Draw the `/ap` mode picker as a centered popup anchored just above the
/// composer (same geometry rules as `render_skill_popup`).
pub fn render_ap_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &ApMenu) {
    // borders + one row per choice + 1 footer hint row.
    let h = (AP_CHOICES.len() as u16 + 4).min(composer_top.max(1));
    let w = 64u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let block = if menu.confirm.is_some() {
        crate::theme::rounded_block_plain()
            .title(" /ap — SAVE AS DEFAULT? y/Enter=global, n=session-only ")
    } else {
        crate::theme::rounded_block_plain().title(" /ap — autopilot 模式 ")
    };
    let mut lines: Vec<Line> = Vec::new();
    for (i, choice) in AP_CHOICES.iter().enumerate() {
        let selected = i == menu.selected;
        let is_current = choice.mode == menu.current;
        let line_style = if selected {
            Style::default()
                .fg(crate::theme::warn_color())
                .add_modifier(Modifier::BOLD)
        } else if is_current {
            Style::default().fg(crate::theme::accent())
        } else {
            Style::default().fg(crate::theme::subtle())
        };
        let key = format!("{:<KEY_W$}", choice.key);
        let mark = if is_current { "← 当前" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!("{key} "), line_style),
            Span::styled(choice.description, line_style),
            Span::styled(mark, Style::default().fg(crate::theme::muted())),
        ]));
    }
    lines.push(Line::styled(
        " ↑/↓ 选择 · Enter 保存 · Esc/Ctrl-D 取消",
        Style::default().fg(crate::theme::muted()),
    ));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left),
        popup,
    );

    if menu.confirm.is_some() {
        render_ap_confirm(f, area, menu);
    }
}

/// Centered overlay shown while the "save as default?" prompt is armed
/// (clone of `model_menu::view::render_save_default_confirm`): drawn on top
/// of the `/ap` popup so the second keystroke is unmistakable.
fn render_ap_confirm(f: &mut Frame, area: Rect, menu: &ApMenu) {
    let w = 62u16.min(area.width.saturating_sub(4));
    let h = 5u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let mode = menu.confirm.unwrap_or(menu.current);
    let key = AP_CHOICES
        .iter()
        .find(|c| c.mode == mode)
        .map(|c| c.key)
        .unwrap_or("off");

    let lines = vec![
        Line::styled(
            format!("将 autopilot 模式设为 {key} 作为全局默认？"),
            Style::default()
                .fg(crate::theme::warn_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::styled(
            " [y]/Enter 全局    [n] 仅本会话    Esc 取消 ",
            Style::default().fg(crate::theme::accent()),
        ),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .block(
                crate::theme::rounded_block_plain()
                    .title(" Save as default? ")
                    .border_style(Style::default().fg(crate::theme::warn_color())),
            )
            .alignment(Alignment::Left),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::{ApMode, Config};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Concatenate one buffer row into a plain string.
    fn row_text(buf: &ratatui::buffer::Buffer, y: u16, w: u16) -> String {
        (0..w)
            .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
            .collect()
    }

    /// Smoke test: the popup renders without panicking and shows the title,
    /// all three mode keys and the `← 当前` mark on the active mode.
    #[test]
    fn popup_renders_title_choices_and_current_mark() {
        crate::theme::set_theme(crate::theme::ThemeKind::Dark);
        let mut config = Config::default();
        config.autopilot.mode = ApMode::Ap;
        let menu = ApMenu::new(&config);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_ap_popup(f, f.area(), 20, &menu))
            .unwrap();
        let buf = terminal.backend().buffer();
        let text: String = (0..buf.area.height)
            .map(|y| row_text(buf, y, buf.area.width))
            .collect::<Vec<_>>()
            .join("\n");
        // Wide CJK glyphs occupy two buffer cells (the second is a blank
        // filler), so match against whitespace-stripped text.
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("/ap"), "title row renders; got: {text}");
        assert!(compact.contains("off"));
        assert!(compact.contains("ap"));
        assert!(compact.contains("review"));
        assert!(compact.contains("当前"), "the active mode is marked");
        assert!(compact.contains("Enter"), "footer hint renders");
    }

    /// The armed confirm prompt redraws the popup title and overlays the
    /// "save as default?" dialog naming the pending mode and the y/n/Esc
    /// hints. Rendered on a tall terminal so the short popup's title row
    /// stays clear of the centered overlay (on a 24-row terminal the
    /// overlay legitimately covers it — the overlay carries its own title).
    #[test]
    fn confirm_overlay_renders_save_as_default_hints() {
        crate::theme::set_theme(crate::theme::ThemeKind::Dark);
        let mut config = Config::default();
        config.autopilot.mode = ApMode::Off;
        let mut menu = ApMenu::new(&config);
        menu.selected = 1; // cursor on ap
        menu.confirm = Some(ApMode::Ap);
        let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
        terminal
            .draw(|f| render_ap_popup(f, f.area(), 34, &menu))
            .unwrap();
        let buf = terminal.backend().buffer();
        let text: String = (0..buf.area.height)
            .map(|y| row_text(buf, y, buf.area.width))
            .collect::<Vec<_>>()
            .join("\n");
        // Wide CJK glyphs occupy two buffer cells (the second is a blank
        // filler), so match against whitespace-stripped text.
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            text.contains("SAVE AS DEFAULT"),
            "confirm title renders; got: {text}"
        );
        assert!(compact.contains("仅本会话"), "session-only hint renders");
        assert!(compact.contains("设为ap"), "the pending mode key is named");
    }
}
