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
            render_status(f, f.area(), false, "", 0, 5000, 80000, 200000, 0);
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

/// High context usage renders the status bar ctx% indicator in red.
#[test]
fn status_bar_ctx_red_at_high_usage() {
    crate::theme::set_theme(crate::theme::ThemeKind::Dark);
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(f, f.area(), false, "", 0, 180000, 200000, 200000, 0);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let area = buf.area;
    let row = row_text(buf, 0, area.width);
    // Use char count (not byte offset) because the status bar now contains
    // multi-byte UTF-8 separators before "ctx".
    let ctx_byte = row.find("ctx").expect("ctx should be present");
    let ctx_col = row[..ctx_byte].chars().count() as u16;
    let cell = buf.cell((ctx_col, 0)).expect("cell at ctx");
    assert_eq!(cell.fg, Color::Red, "high usage should be red; got: {row}");
}
