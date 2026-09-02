use super::*;
use crate::queue_panel::{render_queue_panel, QueueBtn, QueueBtnAction};

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
/// in the rightmost column and window by `scroll`:
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
    assert_eq!(buf[(79, 0)].symbol(), " ", "thumb is a grid-stable blank");
    assert_eq!(buf[(79, 0)].bg, crate::theme::subtle(), "thumb at top");
    assert_eq!(buf[(79, 2)].bg, crate::theme::muted(), "track below thumb");
    // Hit rects stay aligned with the (shifted-left) control strip.
    let del = btns
        .iter()
        .filter(|b| b.seq == 1 && b.action == QueueBtnAction::Delete)
        .collect::<Vec<_>>();
    assert_eq!(del.len(), 1, "one delete button for the oldest row");
    // glyph_hit_rect spans [glyph_x-1, glyph_x]: del_x = 78 → rect starts 77.
    assert_eq!(del[0].rect.x, 77, "delete hit rect covers separator+glyph");
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
    assert_eq!(buf[(79, 0)].symbol(), " ", "track is a grid-stable blank");
    assert_eq!(buf[(79, 0)].bg, crate::theme::muted(), "track above thumb");
    assert_eq!(buf[(79, 2)].bg, crate::theme::subtle(), "thumb at bottom");
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
            // glyph_hit_rect spans [glyph_x-1, glyph_x], so each rect starts
            // one col left of its glyph offset.
            QueueBtnAction::Up => assert_eq!(x, expected[0] - 1, "up glyph col"),
            QueueBtnAction::Down => assert_eq!(x, expected[1] - 1, "down glyph col"),
            QueueBtnAction::Delete => assert_eq!(x, expected[2] - 1, "delete glyph col"),
            QueueBtnAction::Submit => panic!("queue rows never carry a submit button"),
        }
    }
}

/// Render the panel in a `w`×`h` TestBackend; return the emitted hit rects
/// plus a buffer snapshot for glyph-level assertions.
fn render_panel(
    w: u16,
    h: u16,
    steers: &[(i64, String)],
    queues: &[(i64, String)],
    scroll: u32,
) -> (Vec<QueueBtn>, ratatui::buffer::Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, steers, queues, scroll, &mut btns);
        })
        .unwrap();
    (btns, terminal.backend().buffer().clone())
}

/// Symbol rendered at one buffer cell ("" when out of bounds).
fn symbol(buf: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
    buf.cell((x, y))
        .map(|c| c.symbol().to_string())
        .unwrap_or_default()
}

/// Assert a 2-wide rect whose glyph column holds `glyph` and whose left
/// column is the separator space.
fn assert_rect_on_glyph(
    buf: &ratatui::buffer::Buffer,
    b: &QueueBtn,
    action: QueueBtnAction,
    glyph: &str,
) {
    assert_eq!(
        b.rect.width, 2,
        "{action:?} rect must span separator + glyph"
    );
    assert_eq!(
        symbol(buf, b.rect.x + 1, b.rect.y),
        glyph,
        "{action:?} glyph column does not hold the rendered glyph"
    );
    assert_eq!(
        symbol(buf, b.rect.x, b.rect.y),
        " ",
        "{action:?} rect's separator column is not blank"
    );
}

/// Core regression: each recorded hit rect must contain the glyph ratatui
/// actually renders. The old composer width model diverged from
/// unicode-width on dingbats (✓ ✕ ✂ counted as 2 cols, render as 1), so the
/// right-aligning pad came up short and the whole strip — both buttons
/// together — drifted off its rects onto blank cells.
#[test]
fn steer_btn_rects_contain_rendered_glyphs_dingbat_text() {
    let steers: Vec<(i64, String)> = vec![(
        7,
        "\u{2702} \u{2713} \u{2715} \u{2702} fix the dingbat bug".into(),
    )];
    let (btns, buf) = render_panel(80, 5, &steers, &[], 0);
    assert_rect_on_glyph(
        &buf,
        btns.iter()
            .find(|b| b.action == QueueBtnAction::Delete)
            .expect("delete btn"),
        QueueBtnAction::Delete,
        "\u{2715}",
    );
    assert_rect_on_glyph(
        &buf,
        btns.iter()
            .find(|b| b.action == QueueBtnAction::Submit)
            .expect("submit btn"),
        QueueBtnAction::Submit,
        ">",
    );
}

/// Same alignment property for queue rows, with a wide emoji/ZWJ head that
/// only the sequence-aware width model measures correctly.
#[test]
fn queue_btn_rects_contain_rendered_glyphs_emoji_text() {
    let queues: Vec<(i64, String)> = vec![(
        9,
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} then run lint".into(),
    )];
    let (btns, buf) = render_panel(80, 5, &[], &queues, 0);
    assert_rect_on_glyph(
        &buf,
        btns.iter()
            .find(|b| b.action == QueueBtnAction::Up)
            .expect("up btn"),
        QueueBtnAction::Up,
        "\u{25b2}",
    );
    assert_rect_on_glyph(
        &buf,
        btns.iter()
            .find(|b| b.action == QueueBtnAction::Down)
            .expect("down btn"),
        QueueBtnAction::Down,
        "\u{25bc}",
    );
    assert_rect_on_glyph(
        &buf,
        btns.iter()
            .find(|b| b.action == QueueBtnAction::Delete)
            .expect("del btn"),
        QueueBtnAction::Delete,
        "\u{2715}",
    );
}

/// With the scrollbar taking the rightmost column, the strip shifts one
/// column left and its rects must follow — still on the glyphs, never
/// spilling onto the scrollbar itself.
#[test]
fn steer_btn_rects_stay_on_glyphs_with_scrollbar_overflow() {
    let steers: Vec<(i64, String)> = (0..5)
        .map(|i| {
            (
                i,
                format!(
                    "\u{2713} \u{2764}\u{FE0F} \u{2702} steer row {i} — keep the strip aligned"
                ),
            )
        })
        .collect();
    let (btns, buf) = render_panel(80, 5, &steers, &[], 0);
    assert_eq!(btns.len(), 6, "3 visible rows × 2 buttons");
    for b in &btns {
        let glyph = if b.action == QueueBtnAction::Delete {
            "\u{2715}"
        } else {
            ">"
        };
        assert_eq!(
            symbol(&buf, b.rect.x + 1, b.rect.y),
            glyph,
            "{:?} glyph off its rect at x={}",
            b.action,
            b.rect.x
        );
        assert!(
            b.rect.x + b.rect.width <= 79,
            "rect must not cover the scrollbar column"
        );
    }
}
