use super::*;

// ----- Run-duration timer tests -----

#[test]
fn format_run_duration_formats_correctly() {
    assert_eq!(super::format_run_duration(0), "0s");
    assert_eq!(super::format_run_duration(999), "0s");
    assert_eq!(super::format_run_duration(1000), "1s");
    assert_eq!(super::format_run_duration(59000), "59s");
    assert_eq!(super::format_run_duration(60000), "1m0s");
    assert_eq!(super::format_run_duration(119000), "1m59s");
    assert_eq!(super::format_run_duration(120000), "2m0s");
    assert_eq!(super::format_run_duration(3599000), "59m59s");
    assert_eq!(super::format_run_duration(3600000), "1h0m0s");
    assert_eq!(super::format_run_duration(3900000), "1h5m0s");
    assert_eq!(super::format_run_duration(7384000), "2h3m4s");
}

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
            render_status(f, f.area(), false, "", "glm-4.6", "act", 0, 0, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.contains("0s"),
        "zero duration should not be rendered; got: {row}"
    );
}
