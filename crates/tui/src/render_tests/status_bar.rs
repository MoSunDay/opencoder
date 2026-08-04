use super::*;

/// The status bar renders model / agent / dir / ctx but must NOT contain the
/// brand name "opencoder" anywhere (regression guard for the de-branding).
#[test]
fn status_bar_omits_branding() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(f, area, false, "", "glm-4.6", "act", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.to_lowercase().contains("opencoder"),
        "status bar must not contain branding; got: {row}"
    );
    assert!(row.contains("glm-4.6"), "model should appear; got: {row}");
    assert!(
        row.contains("[act]"),
        "agent chip should appear; got: {row}"
    );
}

/// While running, the status bar shows the status text plus the first braille
/// spinner frame, and still omits the brand name.
#[test]
fn status_bar_running_shows_spinner_and_status() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(f, area, true, "thinking", "glm-4.6", "act", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        row.contains("thinking"),
        "status text should appear; got: {row}"
    );
    assert!(
        row.contains('\u{280b}'),
        "first spinner frame should appear; got: {row}"
    );
    assert!(
        !row.to_lowercase().contains("opencoder"),
        "status bar must not contain branding; got: {row}"
    );
}

// ----- Guard: skill badge removed from status bar -----

/// The status bar must NOT render a `skill:` badge (removed per user request
/// — the skill is no longer surfaced in the bottom bar; only the echoed text
/// in the body carries the $name token verbatim).
#[test]
fn status_bar_has_no_skill_badge() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(f, area, false, "", "glm-4.6", "act", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.contains("skill:"),
        "status bar must not contain skill badge; got: {row}"
    );
}

// ----- Guard: steer/queue counters removed from status bar; ctx% present -----

/// The status bar no longer carries the steer/queue counters but DOES show the
/// ctx% indicator (moved from the body's reserved bottom row into the status
/// bar). Guards against accidental re-introduction of steer/queue.
#[test]
fn status_bar_has_no_steer_queue_or_ctx() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(f, area, true, "thinking", "glm-4.6", "act", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.contains("steer:"),
        "status bar must not show steer counter; got: {row}"
    );
    assert!(
        !row.contains("queue:"),
        "status bar must not show queue counter; got: {row}"
    );
    assert!(
        row.contains("ctx"),
        "status bar should show ctx indicator; got: {row}"
    );
}
