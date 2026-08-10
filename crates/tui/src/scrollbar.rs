//! Grid-stable scrollbar geometry and painting shared by TUI surfaces.

use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::Frame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Thumb {
    offset: u16,
    height: u16,
}

/// Compute a proportional thumb using only logical rows.
fn thumb_geometry(track_height: u16, total: usize, visible: usize, scroll: usize) -> Thumb {
    let max_scroll = total.saturating_sub(visible);
    let height = ((visible as u64 * track_height as u64) / total.max(1) as u64).max(1) as u16;
    let height = height.min(track_height);
    let max_offset = track_height.saturating_sub(height);
    let offset = if max_scroll == 0 {
        0
    } else {
        ((scroll.min(max_scroll) as u64 * max_offset as u64) / max_scroll as u64) as u16
    };
    Thumb { offset, height }
}

/// Paint with background-filled blank cells. Unlike block/box-drawing glyphs,
/// a blank always occupies exactly one terminal cell on every font stack.
pub(crate) fn draw(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    visible: usize,
    scroll: usize,
    track_color: Color,
    thumb_color: Color,
) {
    if area.is_empty() || total == 0 {
        return;
    }
    let thumb = thumb_geometry(area.height, total, visible, scroll);
    let x = area.right().saturating_sub(1);
    let buffer = frame.buffer_mut();
    for y in 0..area.height {
        let color = if y >= thumb.offset && y < thumb.offset + thumb.height {
            thumb_color
        } else {
            track_color
        };
        let cell = &mut buffer[(x, area.y + y)];
        cell.reset();
        cell.set_symbol(" ").set_bg(color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn thumb_reaches_both_ends_and_clamps_scroll() {
        assert_eq!(
            thumb_geometry(3, 5, 3, 0),
            Thumb {
                offset: 0,
                height: 1
            }
        );
        assert_eq!(
            thumb_geometry(3, 5, 3, 2),
            Thumb {
                offset: 2,
                height: 1
            }
        );
        assert_eq!(
            thumb_geometry(3, 5, 3, 99),
            Thumb {
                offset: 2,
                height: 1
            }
        );
    }

    #[test]
    fn fully_visible_content_fills_track() {
        assert_eq!(
            thumb_geometry(4, 4, 4, 0),
            Thumb {
                offset: 0,
                height: 4
            }
        );
    }

    #[test]
    fn painter_uses_blank_cells_with_track_and_thumb_backgrounds() {
        let backend = TestBackend::new(4, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, frame.area(), 5, 3, 0, Color::Blue, Color::Yellow);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(3, 0)].symbol(), " ");
        assert_eq!(buffer[(3, 0)].bg, Color::Yellow);
        assert_eq!(buffer[(3, 2)].symbol(), " ");
        assert_eq!(buffer[(3, 2)].bg, Color::Blue);
    }
}
