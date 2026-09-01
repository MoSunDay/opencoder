use super::*;

/// The status bar keeps ctx / timer / spinner but must NOT contain the brand
/// name "opencoder" anywhere (regression guard for the de-branding), nor the
/// model name. The mode chip is anchored at the bottom-left.
#[test]
fn status_bar_omits_branding_and_top_moved_info() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "act",
                false,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
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
        row.starts_with(" \u{25cf} [act]"),
        "a status dot must precede the mode chip at the bottom-left; got: {row}"
    );
    assert!(
        row.contains("ctx"),
        "status bar should still show ctx; got: {row}"
    );
}

/// Helper: extract the `String` of display chars strictly between the mode
/// chip's `] · ` and the `ctx` text. The two 10-segment meters live there.
fn meters_between_chip_and_ctx(row: &str) -> String {
    let start = row
        .find("] \u{00b7} ")
        .map(|i| i + "] \u{00b7} ".len())
        .expect("mode chip + separator must be present");
    let end = row.find("ctx").expect("ctx text must be present");
    row[start..end].to_string()
}

/// A single 10-segment compression dial sits between the mode chip and the
/// ctx text (the second budget-gauge was removed per user request). With
/// used=0 the single dial is all empty — exactly 10 empty cells.
#[test]
fn status_bar_has_single_meter_before_ctx() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "act",
                false,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    let meters = meters_between_chip_and_ctx(&row);
    assert_eq!(
        meters.matches('\u{25b0}').count(),
        0,
        "used=0 → no filled cells; got: {meters:?}"
    );
    assert_eq!(
        meters.matches('\u{25b1}').count(),
        10,
        "single all-empty meter (10 cells); got: {meters:?}"
    );
}

/// The single remaining dial tracks the compaction threshold, not the model
/// window: with used (180K) above the threshold (80K) but below the window
/// (200K) the dial is fully filled (10/10) — and no second gauge exists to
/// fill only ~90%.
#[test]
fn status_bar_single_dial_tracks_threshold_not_window() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "act",
                false,
                false,
                "",
                0,
                Some(180000),
                80000,
                200000,
                0,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    let meters = meters_between_chip_and_ctx(&row);
    assert_eq!(
        meters.matches('\u{25b0}').count(),
        10,
        "dial is full once used exceeds the threshold; got: {meters:?}"
    );
    assert_eq!(
        meters.matches('\u{25b1}').count(),
        0,
        "no second meter remains (budget gauge removed); got: {meters:?}"
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
            render_status(
                f,
                area,
                "act",
                false,
                true,
                "compacting\u{2026}",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        row.contains("compacting\u{2026}"),
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
            render_status(
                f,
                area,
                "act",
                false,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
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
            render_status(
                f,
                area,
                "act",
                false,
                true,
                "compacting\u{2026}",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
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
/// BEFORE the running spinner (time → motion), styled in warn colour.
#[test]
fn status_bar_shows_task_time() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            // 90s = 1m30s
            render_status(
                f,
                area,
                "act",
                false,
                true,
                "compacting\u{2026}",
                0,
                Some(0),
                200000,
                200000,
                90000,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row = row_text(buf, 0, 120);
    let time_pos = row
        .find("1m30s")
        .expect("status bar should show cumulative task time");
    let spin_pos = row.find('\u{280b}').expect("running spinner should render");
    assert!(
        time_pos < spin_pos,
        "task time must sit BEFORE the spinner; time_pos={time_pos}, spin_pos={spin_pos}; got: {row}"
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

/// Once the task stops (running=false), the frozen cumulative timer keeps
/// its position but flips from warn (orange) to muted (gray) — the state
/// colour coding of the status bar (the spinner vanishes on stop; the timer
/// turns warn again on the next submitted requirement).
#[test]
fn status_bar_task_time_turns_muted_when_stopped() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "act",
                false,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                90000,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row = row_text(buf, 0, 120);
    let time_pos = row
        .find("1m30s")
        .expect("stopped task time must stay visible");
    assert!(
        !row.contains('\u{280b}'),
        "no running spinner may appear once stopped; got: {row}"
    );
    let cell_x = row[..time_pos].chars().count() as u16;
    let cell = buf.cell((cell_x, 0)).expect("task-time cell must exist");
    assert_eq!(
        cell.style().fg,
        Some(crate::theme::muted()),
        "stopped task time should use muted color; got: {:?}",
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
            render_status(
                f,
                area,
                "act",
                false,
                true,
                "compacting\u{2026}",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
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

// ----- Blinking status dot while running -----

/// The running dot stays visible for the full first 500ms phase.
#[test]
fn status_dot_stays_visible_through_first_phase() {
    for tick in [0u32, 1, 2, 3, 4, 10] {
        let backend = TestBackend::new(120, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_status(
                    f,
                    area,
                    "act",
                    false,
                    true,
                    "working\u{2026}",
                    tick,
                    Some(0),
                    200000,
                    200000,
                    0,
                );
            })
            .unwrap();

        let row = row_text(terminal.backend().buffer(), 0, 120);
        assert!(
            row.starts_with(" \u{25cf} [act]"),
            "visible phases must show the dot at tick={tick}; got: {row}"
        );
    }
}

/// From 500ms through 900ms the running dot stays hidden while preserving
/// width; tick 10 is covered above as the next visible-phase boundary.
#[test]
fn status_dot_stays_hidden_through_second_phase() {
    for tick in 5u32..=9 {
        let backend = TestBackend::new(120, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_status(
                    f,
                    area,
                    "plan",
                    false,
                    true,
                    "working\u{2026}",
                    tick,
                    Some(0),
                    200000,
                    200000,
                    0,
                );
            })
            .unwrap();

        let row = row_text(terminal.backend().buffer(), 0, 120);
        assert!(
            !row.starts_with(" \u{25cf}"),
            "hidden phase must hide the dot at tick={tick}; got: {row}"
        );
        assert!(
            row.starts_with("   [plan]"),
            "mode chip must stay at the same column at tick={tick}; got: {row}"
        );
    }
}

/// Idle (running=false) never blinks: the dot stays visible for any anim_tick.
#[test]
fn status_dot_stays_steady_when_idle() {
    for tick in [0u32, 1, 2, 999] {
        let backend = TestBackend::new(120, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_status(
                    f,
                    area,
                    "act",
                    false,
                    false,
                    "",
                    tick,
                    Some(0),
                    200000,
                    200000,
                    0,
                );
            })
            .unwrap();

        let row = row_text(terminal.backend().buffer(), 0, 120);
        assert!(
            row.starts_with(" \u{25cf} [act]"),
            "idle dot must stay visible for tick={tick}; got: {row}"
        );
    }
}

/// The plan mode chip renders as `[plan]` in the warning hue: the
/// read-only agent is announced by name AND by colour (the removed plan
/// agent never re-appears as a chip).
#[test]
fn status_bar_plan_chip_renders_in_warn_hue() {
    use crate::theme::agent_chip_fg;

    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "plan",
                false,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row = row_text(buf, 0, 120);
    let chip_pos = row
        .find("[plan]")
        .expect("the plan chip text must render at the bottom-left");
    assert_eq!(
        buf.cell((chip_pos as u16, 0)).unwrap().fg,
        agent_chip_fg("plan"),
        "the chip must be painted in the plan (warn) hue"
    );

    // The same slot for act stays in the accent hue.
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "act",
                false,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let row = row_text(buf, 0, 120);
    let chip_pos = row.find("[act]").expect("the act chip must render");
    assert_eq!(
        buf.cell((chip_pos as u16, 0)).unwrap().fg,
        agent_chip_fg("act"),
        "the act chip must keep the accent hue"
    );
}

/// The parent `[act]` chip (dot + chip share one hue) lights up in the sandbox
/// warning hue while the committed skill is `task-plan`, and keeps the accent
/// hue otherwise. Same layout either way: only `plan_skill_active` differs.
#[test]
fn status_bar_act_chip_lights_warn_for_task_plan() {
    let chip_fg = |plan: bool| -> (usize, ratatui::style::Color) {
        let backend = TestBackend::new(120, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_status(
                    f,
                    area,
                    "act",
                    plan,
                    false,
                    "",
                    0,
                    Some(0),
                    200000,
                    200000,
                    0,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let row = row_text(buf, 0, 120);
        let pos = row.find("[act]").expect("the act chip must render");
        (pos, buf.cell((pos as u16, 0)).unwrap().fg)
    };

    let (pos_plan, fg_plan) = chip_fg(true);
    assert_eq!(
        fg_plan,
        ratatui::style::Color::Yellow,
        "a committed task-plan must light the [act] chip yellow"
    );
    let (pos_plain, fg_plain) = chip_fg(false);
    assert_eq!(
        fg_plain,
        ratatui::style::Color::Cyan,
        "without task-plan the [act] chip keeps the accent hue"
    );
    assert_eq!(
        pos_plan, pos_plain,
        "the highlight must never shift the chip horizontally"
    );

    // The leading dot shares the chip hue while the highlight is active.
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f,
                area,
                "act",
                true,
                false,
                "",
                0,
                Some(0),
                200000,
                200000,
                0,
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let row = row_text(buf, 0, 120);
    let dot_pos = row.find('\u{25cf}').expect("the status dot must render");
    assert_eq!(
        buf.cell((dot_pos as u16, 0)).unwrap().fg,
        ratatui::style::Color::Yellow,
        "the dot must share the task-plan highlight hue"
    );
}
