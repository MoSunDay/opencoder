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
            render_queue_panel(f, area, &steers, &queues, &mut btns);
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
            render_queue_panel(f, area, &steers, &queues, &mut btns);
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
