use super::*;

#[test]
fn status_chip_width_accounts_for_wide_emoji() {
    // Two emoji = 4 display columns but only 2 chars. With the old
    // chars().count() the chip rectangle was 2 columns too narrow, so the
    // second emoji was clipped out of the render entirely.
    let backend = TestBackend::new(60, 1);
    let mut terminal = Terminal::new(backend).unwrap();
    let text = "📋🎉";
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 60, 1);
            render_status_chip(f, area, text, crate::theme::ok_color());
        })
        .unwrap();
    let row = row_text(terminal.backend().buffer(), 0, 60);
    assert!(row.contains('📋'), "first emoji missing; got: {row}");
    assert!(
        row.contains('🎉'),
        "second emoji was clipped — chip width did not account for wide chars; got: {row}"
    );
}

#[test]
fn status_bar_shows_ctx_percent() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(f, f.area(), "act", false, "", 0, 5000, 80000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        row.contains("ctx"),
        "status bar should show ctx; got: {row}"
    );
    assert!(
        row.contains('%'),
        "status bar should show percent; got: {row}"
    );
    assert!(
        row.contains("5K"),
        "should show compact used tokens; got: {row}"
    );
    assert!(
        row.contains("200K"),
        "ctx denominator should be the model window, not the threshold; got: {row}"
    );
    assert!(
        !row.contains("80K"),
        "ctx denominator must NOT show the compaction threshold; got: {row}"
    );
}

/// Colour-split regression: only the meter bar + percent value follow the
/// threshold colour; the `thr` label and `ctx (used/limit)` counts keep the
/// normal text colour (Say-body colour) no matter how high the usage climbs.
#[test]
fn status_bar_colors_split_between_meter_and_labels() {
    crate::theme::set_theme(crate::theme::ThemeKind::Dark);
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            // 180K/200K threshold → 90% → err colour for the meter.
            render_status(f, f.area(), "act", false, "", 0, 180000, 200000, 200000, 0);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let area = buf.area;
    let row = row_text(buf, 0, area.width);
    // Use char count (not byte offset) because the status bar contains
    // multi-byte UTF-8 separators before these markers.
    let col_of = |needle: &str| -> u16 {
        let b = row.find(needle).expect("marker must be present");
        row[..b].chars().count() as u16
    };
    let expected = crate::theme::err_color();
    let normal = crate::theme::text();
    let meter_cell = buf
        .cell((col_of("\u{25b0}"), 0))
        .expect("cell at first filled meter segment");
    assert_eq!(
        meter_cell.fg, expected,
        "meter bar must use the threshold colour; got: {row}"
    );
    let pct_cell = buf
        .cell((col_of("%"), 0))
        .expect("cell at percent sign");
    assert_eq!(
        pct_cell.fg, expected,
        "percent value must use the threshold colour; got: {row}"
    );
    let thr_cell = buf.cell((col_of("thr"), 0)).expect("cell at thr label");
    assert_eq!(
        thr_cell.fg, normal,
        "thr label must keep normal text colour; got: {row}"
    );
    let ctx_cell = buf.cell((col_of("ctx"), 0)).expect("cell at ctx label");
    assert_eq!(
        ctx_cell.fg, normal,
        "ctx counts must keep normal text colour (ratio-to-total, not threshold-coloured); got: {row}"
    );
}
