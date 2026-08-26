//! Attachment badge rendering — one line per pending image with a clickable
//! ✕ (U+2715) delete button at the row end. The composer shifts its input
//! area down by the number of badge rows actually rendered, so multi-image
//! pastes keep every row individually dismissible.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::composer;
use crate::render::MouseHits;
use crate::theme;

/// A clickable ✕ delete button for one attachment row. `index` indexes
/// `pending_images` at render time; rects are recomputed every frame and a
/// single click removes exactly one image, so the index stays valid.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AttachDelBtn {
    pub index: usize,
    pub rect: Rect,
}

/// Render one badge line per pending image inside `inner`: `📎 {filename}`
/// (warn colour) truncated to leave the last cell free, with a right-aligned
/// ✕ button at `inner.x + inner.width - 1`. Each ✕ is registered in
/// `hits.attach_del_btns`; rows beyond `inner.height` are not drawn. Returns
/// the number of rows actually used (0 when there is nothing to show).
pub(crate) fn render_attach_badge(
    f: &mut Frame,
    inner: Rect,
    pending_images: &[(String, String)],
    hits: &mut MouseHits,
) -> u16 {
    if pending_images.is_empty() || inner.height == 0 {
        return 0;
    }
    let rows = (pending_images.len() as u16).min(inner.height);
    // Reserve the last cell for ✕; `truncate_to_width` keeps the whole label
    // (including ellipsis) within the remaining columns.
    let label_cap = inner.width.saturating_sub(1) as usize;
    let del_x = inner.x + inner.width.saturating_sub(1);
    for (i, (_, name)) in pending_images.iter().take(rows as usize).enumerate() {
        let y = inner.y + i as u16;
        let label = composer::truncate_to_width(&format!("\u{1f4ce} {name}"), label_cap);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                label,
                Style::default().fg(theme::warn_color()),
            ))),
            Rect::new(inner.x, y, inner.width, 1),
        );
        let del_rect = Rect::new(del_x, y, 1, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "\u{2715}",
                Style::default()
                    .fg(theme::warn_color())
                    .add_modifier(Modifier::BOLD),
            ))),
            del_rect,
        );
        hits.attach_del_btns.push(AttachDelBtn {
            index: i,
            rect: del_rect,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
        let mut s = String::new();
        for x in 0..width {
            if let Some(cell) = buf.cell((x, y)) {
                s.push_str(cell.symbol());
            }
        }
        s
    }

    fn imgs(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| ("data:image/png;base64,x".to_string(), n.to_string()))
            .collect()
    }

    /// One badge line per attachment, ✕ right-aligned at the last inner
    /// cell, one hit rect registered per row.
    #[test]
    fn badge_rows_and_del_buttons() {
        let backend = TestBackend::new(20, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = MouseHits::default();
        terminal
            .draw(|f| {
                let rows = render_attach_badge(
                    f,
                    Rect::new(1, 1, 18, 4),
                    &imgs(&["a.png", "b.png"]),
                    &mut hits,
                );
                assert_eq!(rows, 2, "one row per attachment");
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let row1 = row_text(buf, 1, 20);
        let row2 = row_text(buf, 2, 20);
        assert!(
            row1.contains('\u{1f4ce}') && row1.contains("a.png"),
            "row1: {row1}"
        );
        assert!(
            row2.contains('\u{1f4ce}') && row2.contains("b.png"),
            "row2: {row2}"
        );
        // ✕ right-aligned at inner.x + inner.width - 1 = 1 + 18 - 1 = 18.
        assert_eq!(buf[(18, 1)].symbol(), "\u{2715}", "row-1 ✕ at last cell");
        assert_eq!(buf[(18, 2)].symbol(), "\u{2715}", "row-2 ✕ at last cell");
        assert_eq!(hits.attach_del_btns.len(), 2);
        assert_eq!(hits.attach_del_btns[0].index, 0);
        assert_eq!(hits.attach_del_btns[0].rect, Rect::new(18, 1, 1, 1));
        assert_eq!(hits.attach_del_btns[1].index, 1);
        assert_eq!(hits.attach_del_btns[1].rect, Rect::new(18, 2, 1, 1));
    }

    /// Long filenames are truncated with an ellipsis; the ✕ cell is never
    /// overwritten by the label.
    #[test]
    fn filename_truncated_keeping_del_cell() {
        let backend = TestBackend::new(12, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = MouseHits::default();
        let long = "very-long-filename-with-many-chars.png";
        terminal
            .draw(|f| {
                let rows =
                    render_attach_badge(f, Rect::new(1, 0, 10, 2), &imgs(&[long]), &mut hits);
                assert_eq!(rows, 1);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        // inner.x + inner.width - 1 = 1 + 10 - 1 = 10.
        assert_eq!(buf[(10, 0)].symbol(), "\u{2715}", "✕ owns the last cell");
        let row = row_text(buf, 0, 11);
        assert!(!row.contains(long), "long name must be truncated: {row}");
        assert!(row.contains('\u{2026}'), "ellipsis marker: {row}");
    }

    /// More attachments than inner rows: only `inner.height` lines are drawn
    /// and registered.
    #[test]
    fn rows_capped_at_inner_height() {
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = MouseHits::default();
        let names: Vec<String> = (0..5).map(|i| format!("f{i}.png")).collect();
        let imgs: Vec<(String, String)> =
            names.iter().map(|n| ("x".to_string(), n.clone())).collect();
        terminal
            .draw(|f| {
                let rows = render_attach_badge(f, Rect::new(0, 0, 20, 2), &imgs, &mut hits);
                assert_eq!(rows, 2, "drawn rows capped at inner.height");
            })
            .unwrap();
        assert_eq!(hits.attach_del_btns.len(), 2);
        assert_eq!(hits.attach_del_btns[1].index, 1);
    }

    /// Empty image list renders nothing and registers no buttons.
    #[test]
    fn empty_images_renders_nothing() {
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut hits = MouseHits::default();
        terminal
            .draw(|f| {
                let rows = render_attach_badge(f, Rect::new(0, 0, 20, 4), &[], &mut hits);
                assert_eq!(rows, 0);
            })
            .unwrap();
        assert!(hits.attach_del_btns.is_empty());
    }
}
