//! Rendering for the `@` file-mention picker — a dropdown anchored just
//! above the composer (same geometry as `command.rs::render_command_popup`).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::state::FileMenu;
use crate::theme;

/// Draw the picker: box + rows + `@query` footer, bottom edge sitting above
/// the composer's top row.
pub fn render_file_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &FileMenu) {
    // Box = 2 borders + content rows; +1 row for the query footer below.
    let want_box = menu.visible_count() as u16 + 4;
    let want_total = want_box.saturating_add(1);
    let avail = composer_top.max(1);
    let total = want_total.min(avail);
    let h = total.saturating_sub(1).max(3);
    let w = 72u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(total);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let block = theme::rounded_block(
        "@files (\u{2191}/\u{2193} move, type to filter, Enter/Tab=insert, Esc=cancel)",
    );

    let items: Vec<ListItem> = menu
        .visible_entries()
        .map(|e| {
            let label = if e.is_dir {
                format!("{}/", e.rel)
            } else {
                e.rel.clone()
            };
            ListItem::new(Line::from(Span::styled(
                label,
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD),
            )))
        })
        .collect();

    let items = if items.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  no matching file",
            Style::default().fg(theme::muted()),
        )))]
    } else {
        items
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::list_highlight())
        .highlight_symbol("\u{276f} ");

    let mut state = ListState::default();
    if menu.visible_count() > 0 {
        state.select(Some(menu.selected_row()));
    }
    f.render_stateful_widget(list, popup, &mut state);

    // Query footer (mirrors the command popup).
    let footer = Rect::new(
        popup.x,
        popup.bottom(),
        popup.width,
        1u16.min(area.height.saturating_sub(popup.bottom())),
    );
    if footer.height > 0 {
        let line = Line::from(vec![
            Span::styled(" @", Style::default().fg(theme::muted())),
            Span::styled(
                menu.query().to_string(),
                Style::default()
                    .fg(theme::warn_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("_"),
        ]);
        f.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), footer);
    }
}
