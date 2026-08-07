use super::*;

// ----- Run-duration timer tests -----

/// When running with a non-zero run_ms, the status bar shows the duration.
#[test]
fn status_bar_shows_run_duration() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(
                f,
                f.area(),
                true,
                "working",
                "glm-4.6",
                "act",
                0,
                5000,
                200000,
                200000,
                42000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        row.contains("42s"),
        "status bar should show run duration; got: {row}"
    );
}

/// When run_ms is 0, no duration is shown (idle session).
#[test]
fn status_bar_hides_duration_when_zero() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(f, f.area(), false, "", "glm-4.6", "act", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.contains("0s"),
        "zero duration should not be rendered; got: {row}"
    );
}

/// While running, the run-duration timer appears at the *tail* of the status
/// line — after the spinner and status text, not between ctx and status.
#[test]
fn status_bar_timer_at_tail_after_status() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(
                f,
                f.area(),
                true,
                "working",
                "glm-4.6",
                "act",
                0,
                5000,
                200000,
                200000,
                42000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    let timer_pos = row.find("42s");
    let status_pos = row.find("working");
    assert!(
        timer_pos.is_some() && status_pos.is_some(),
        "both timer and status text should be present; got: {row}"
    );
    assert!(
        timer_pos > status_pos,
        "timer must appear after status text (at the tail of the line); got: {row}"
    );
}
