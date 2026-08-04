use super::*;
use crate::queue_panel::render_queue_panel;

/// The queue panel renders steer items with the `↳ steer` prefix and queue
/// items with `[queued]`, and caps display at 3 rows.
#[test]
fn queue_panel_renders_steer_and_queue_rows() {
    use crate::queue_panel::QueueBtn;
    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    let steers: Vec<(i64, String)> = vec![(1, "fix bug".into())];
    let queues: Vec<(i64, String)> = vec![(2, "run lint".into())];
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &steers, &queues, 0, &mut btns);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row0 = row_text(buf, 0, 80);
    let row1 = row_text(buf, 1, 80);
    assert!(
        row0.contains("steer") && row0.contains("fix bug"),
        "steer row missing: {row0}"
    );
    assert!(
        row1.contains("queued") && row1.contains("run lint"),
        "queue row missing: {row1}"
    );
}

/// Steer rows register two hit-rects (Delete + Submit); queue rows register
/// three (Up + Down + Delete). Steer rows are now clickable (seq: Some).
#[test]
fn queue_panel_registers_correct_btns_for_steer_and_queue() {
    use crate::queue_panel::{QueueBtn, QueueBtnAction};
    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    let steers: Vec<(i64, String)> = vec![(10, "fix bug".into())];
    let queues: Vec<(i64, String)> = vec![(20, "run lint".into())];
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &steers, &queues, 0, &mut btns);
        })
        .unwrap();

    // Steer row (seq=10) should have Delete + Submit.
    let steer_btns: Vec<_> = btns.iter().filter(|b| b.seq == 10).collect();
    assert_eq!(steer_btns.len(), 2, "steer row should have 2 buttons");
    assert!(
        steer_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Delete),
        "steer row must have a Delete button"
    );
    assert!(
        steer_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Submit),
        "steer row must have a Submit button"
    );
    // Steer row must NOT have Up or Down.
    assert!(
        !steer_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Up || b.action == QueueBtnAction::Down),
        "steer row must not have Up/Down buttons"
    );

    // Queue row (seq=20) should have Up + Down + Delete.
    let queue_btns: Vec<_> = btns.iter().filter(|b| b.seq == 20).collect();
    assert_eq!(queue_btns.len(), 3, "queue row should have 3 buttons");
    assert!(
        queue_btns.iter().any(|b| b.action == QueueBtnAction::Up),
        "queue row must have an Up button"
    );
    assert!(
        queue_btns.iter().any(|b| b.action == QueueBtnAction::Down),
        "queue row must have a Down button"
    );
    assert!(
        queue_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Delete),
        "queue row must have a Delete button"
    );
    // Queue row must NOT have Submit.
    assert!(
        !queue_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Submit),
        "queue row must not have a Submit button"
    );
}

/// Overflowing panels (more entries than the 3-row viewport) draw a scrollbar
/// in the rightmost column (track `│` / thumb `█`) and window by `scroll`:
/// `scroll == 0` pins to the oldest (top) entries, `scroll == max_scroll`
/// shows the newest. The thumb tracks `scroll`, so it sits at the top when
/// pinned and at the bottom when scrolled to the end.
#[test]
fn queue_panel_overflow_windows_and_scrollbar() {
    use crate::queue_panel::{QueueBtn, QueueBtnAction};
    let backend = TestBackend::new(80, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    let queues: Vec<(i64, String)> = vec![
        (1, "oldest".into()),
        (2, "older".into()),
        (3, "middle".into()),
        (4, "newer".into()),
        (5, "newest".into()),
    ];

    // scroll = 0 → pinned to top (oldest): rows are oldest / older / middle.
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &[], &queues, 0, &mut btns);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let row0 = row_text(buf, 0, 80);
    let row2 = row_text(buf, 2, 80);
    assert!(row0.contains("oldest"), "top-anchored window: {row0}");
    assert!(row2.contains("middle"), "top-anchored window: {row2}");
    assert!(
        !row0.contains("newer") && !row0.contains("newest"),
        "newer entries must be hidden at scroll=0: {row0}"
    );
    // Scrollbar: thumb sits at the top when pinned to the oldest window.
    assert_eq!(row0.chars().last(), Some('\u{2588}'), "thumb at top");
    assert_eq!(row2.chars().last(), Some('\u{250a}'), "track below thumb");
    // Hit rects stay aligned with the (shifted-left) control strip.
    let del = btns
        .iter()
        .filter(|b| b.seq == 1 && b.action == QueueBtnAction::Delete)
        .collect::<Vec<_>>();
    assert_eq!(del.len(), 1, "one delete button for the oldest row");
    assert_eq!(del[0].rect.x, 78, "delete glyph 1 col left of scrollbar");
    assert_eq!(del[0].rect.y, 0);

    // scroll = max_scroll (2) → newest entries visible, thumb at the bottom.
    let mut btns2: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &[], &queues, 2, &mut btns2);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let row0 = row_text(buf, 0, 80);
    let row2 = row_text(buf, 2, 80);
    assert!(row0.contains("middle"), "scrolled window: {row0}");
    assert!(row2.contains("newest"), "scrolled window: {row2}");
    assert_eq!(row0.chars().last(), Some('\u{250a}'), "track above thumb");
    assert_eq!(row2.chars().last(), Some('\u{2588}'), "thumb at bottom");
}

/// With the scrollbar taking the rightmost column, every queue-row control
/// glyph shifts one column left — and its hit rect follows exactly, so clicks
/// on ▲/▼/✕ land on the visible glyphs.
#[test]
fn queue_panel_overflow_hit_rects_track_shifted_glyphs() {
    use crate::queue_panel::{btn_x_offsets, QueueBtn, QueueBtnAction};
    let backend = TestBackend::new(80, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    let queues: Vec<(i64, String)> = vec![
        (1, "a".into()),
        (2, "b".into()),
        (3, "c".into()),
        (4, "d".into()),
    ];
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &[], &queues, 0, &mut btns);
        })
        .unwrap();

    // Overflow: content width = 80 - 1 (scrollbar column) → offsets shift
    // left by exactly one relative to the non-overflow geometry.
    let expected = btn_x_offsets(79);
    assert!(!btns.is_empty(), "overflow rows must stay clickable");
    for b in &btns {
        let x = b.rect.x; // area.x == 0 in this terminal
        match b.action {
            QueueBtnAction::Up => assert_eq!(x, expected[0], "up glyph col"),
            QueueBtnAction::Down => assert_eq!(x, expected[1], "down glyph col"),
            QueueBtnAction::Delete => assert_eq!(x, expected[2], "delete glyph col"),
            QueueBtnAction::Submit => panic!("queue rows never carry a submit button"),
        }
    }
}
