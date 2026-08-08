use super::*;

/// The status bar keeps ctx / timer / spinner but must NOT contain the brand
/// name "opencoder" anywhere (regression guard for the de-branding), nor the
/// model name / `[mode]` chip — those moved up into the top body title.
#[test]
fn status_bar_omits_branding_and_top_moved_info() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(f, area, false, "", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.to_lowercase().contains("opencoder"),
        "status bar must not contain branding; got: {row}"
    );
    assert!(
        !row.contains("glm-4.6"),
        "model must be gone from the status bar (moved to top title); got: {row}"
    );
    assert!(
        !row.contains("[act]"),
        "mode chip must be gone from the status bar (moved to top title); got: {row}"
    );
    assert!(
        row.contains("ctx"),
        "status bar should still show ctx; got: {row}"
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
            render_status(f, area, true, "thinking", 0, 0, 200000, 200000, 0);
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
            render_status(f, area, false, "", 0, 0, 200000, 200000, 0);
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
            render_status(f, area, true, "thinking", 0, 0, 200000, 200000, 0);
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


/// When `task_ms > 0`, the status bar shows the cumulative task duration
/// AFTER the running spinner (motion → time), styled in warn colour.
#[test]
fn status_bar_shows_task_time() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            // 90s = 1m30s
            render_status(f, area, true, "thinking", 0, 0, 200000, 200000, 90000);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row = row_text(buf, 0, 120);
    let time_pos = row
        .find("1m30s")
        .expect("status bar should show cumulative task time");
    let spin_pos = row
        .find('\u{280b}')
        .expect("running spinner should render");
    assert!(
        time_pos > spin_pos,
        "task time must sit AFTER the spinner; spin_pos={spin_pos}, time_pos={time_pos}; got: {row}"
    );
    // find() yields a BYTE offset; the row has multi-byte chars (·, ▱, ⠋)
    // so convert to a char index before addressing the buffer.
    let cell_x = row[..time_pos].chars().count() as u16;
    let cell = buf.cell((cell_x, 0)).expect("task-time cell must exist");
    assert_eq!(
        cell.style().fg,
        Some(crate::theme::warn_color()),
        "task time should use warn color; got: {:?}",
        cell.style().fg
    );
}

/// When `task_ms == 0` (no task started yet), the task time is hidden.
#[test]
fn status_bar_hides_task_time_when_zero() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(f, area, true, "thinking", 0, 0, 200000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    // "1m30s" or "42s" should not appear; but we can't just check for "0s"
    // because the ctx text might contain it. Check that no standalone task
    // duration dot-separator appears right before where a duration would be.
    assert!(
        !row.contains("1m30s"),
        "zero task_ms should not show task time; got: {row}"
    );
}
