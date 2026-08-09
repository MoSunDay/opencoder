//! Rendering for the keymap modal popup (Ctrl+H).

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::keymap_menu::help;
use crate::keymap_menu::state::{Focus, KeymapMenu};
use crate::theme;

/// Render the keymap popup as a centered modal.
pub fn render_keymap_popup(f: &mut Frame, area: Rect, menu: &KeymapMenu) {
    let rows = menu.len() as u16;
    let want_h = 3 + rows + 2; // border-top + rows + footer + button-bar
    let h = want_h.min(area.height.saturating_sub(2));
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let block = theme::rounded_block("Keyboard Shortcuts");

    let sel_st = Style::default()
        .fg(theme::accent())
        .add_modifier(Modifier::BOLD);
    let val_st = Style::default().fg(theme::text());
    let dim_st = Style::default().fg(theme::muted());
    let cap_st = Style::default()
        .fg(theme::warn_color())
        .add_modifier(Modifier::BOLD);

    let list_focused = menu.focus() == Focus::List;
    // When buttons have focus, dim the list marker so the highlight reads
    // as "on the buttons" rather than "on a row".
    let marker_sel_st = if list_focused { sel_st } else { dim_st };

    let mut lines: Vec<Line> = Vec::new();
    for (i, (key, label, spec)) in menu.entries().iter().enumerate() {
        let is_sel = i == menu.selected;
        let marker = if is_sel { "❯ " } else { "  " };
        let st = if is_sel && list_focused {
            sel_st
        } else {
            val_st
        };

        let mut spans = vec![
            Span::styled(marker, if is_sel { marker_sel_st } else { val_st }),
            Span::styled(format!("{:<7} ", spec), st),
            Span::styled(label.clone(), if is_sel { st } else { dim_st }),
        ];

        if is_sel && menu.capturing {
            spans[1] = Span::styled("Press a key...  ", cap_st);
        }
        let _ = key; // key is the config key, not shown in the row
        lines.push(Line::from(spans));
    }

    // --- Footer hint ---
    lines.push(Line::from(Span::styled(
        " Enter: rebind   Ctrl+R: reset   Tab: buttons   Esc: close   Ctrl+D: quit",
        dim_st,
    )));

    // --- Button bar ---
    let btn_sel_st = Style::default()
        .fg(theme::accent())
        .add_modifier(Modifier::BOLD);
    let btn_dim_st = Style::default().fg(theme::muted());

    let btn_focused = menu.focus() == Focus::Buttons;
    let exit_sel = btn_focused && menu.selected_button() == 0;
    let reset_sel = btn_focused && menu.selected_button() == 1;
    let help_sel = btn_focused && menu.selected_button() == 2;

    let exit_st = if exit_sel { btn_sel_st } else { btn_dim_st };
    let reset_st = if reset_sel { btn_sel_st } else { btn_dim_st };
    let help_st = if help_sel { btn_sel_st } else { btn_dim_st };

    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("< 退出 >", exit_st),
        Span::raw("   "),
        Span::styled("< 恢复默认 >", reset_st),
        Span::raw("   "),
        Span::styled("< 帮助 >", help_st),
        Span::raw(" "),
    ]));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        popup,
    );

    // --- Help overlay on top ---
    if menu.help_open() {
        help::render_help_overlay(f, area, menu.help_scroll());
    }

    // --- Confirm-reset dialog (topmost) ---
    if menu.confirm_reset_open() {
        render_confirm_reset_overlay(f, area);
    }
}

/// Render the reset-confirmation dialog on top of the keymap modal.
fn render_confirm_reset_overlay(f: &mut Frame, area: Rect) {
    let w = 52u16.min(area.width.saturating_sub(4));
    let h = 7u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title_st = Style::default()
        .fg(theme::warn_color())
        .add_modifier(Modifier::BOLD);
    let hint_st = Style::default().fg(theme::muted());

    let lines = vec![
        Line::from(Span::styled("确认将所有快捷键恢复为默认值？", title_st)),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter/Y] 确认", Style::default().fg(theme::accent())),
            Span::raw("   "),
            Span::styled("[Esc/N] 取消", hint_st),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines).block(theme::rounded_block("恢复默认")),
        popup,
    );
}
