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
            render_status(
                f,
                f.area(),
                "act",
                false,
                "",
                0,
                Some(5000),
                80000,
                200000,
                0,
            );
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
/// threshold colour; the `thr` label and `ctx (used/limit)` counts use the
/// bold bright-blue status label colour no matter how high the usage climbs.
#[test]
fn status_bar_colors_split_between_meter_and_labels() {
    crate::theme::set_theme(crate::theme::ThemeKind::Dark);
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            // 180K/200K threshold → 90% → err colour for the meter.
            render_status(
                f,
                f.area(),
                "act",
                false,
                "",
                0,
                Some(180000),
                200000,
                200000,
                0,
            );
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
    let label = crate::theme::status_label_color();
    let meter_cell = buf
        .cell((col_of("\u{25b0}"), 0))
        .expect("cell at first filled meter segment");
    assert_eq!(
        meter_cell.fg, expected,
        "meter bar must use the threshold colour; got: {row}"
    );
    let pct_cell = buf.cell((col_of("%"), 0)).expect("cell at percent sign");
    assert_eq!(
        pct_cell.fg, expected,
        "percent value must use the threshold colour; got: {row}"
    );
    let thr_cell = buf.cell((col_of("thr"), 0)).expect("cell at thr label");
    assert_eq!(
        thr_cell.fg, label,
        "thr label must stay bright blue; got: {row}"
    );
    let ctx_cell = buf.cell((col_of("ctx"), 0)).expect("cell at ctx label");
    assert_eq!(
        ctx_cell.fg, label,
        "ctx counts must stay bright blue (ratio-to-total, not threshold-coloured); got: {row}"
    );
}

/// `ctx (used/limit)` resolution: the provider-truth `total_tokens` is used
/// verbatim; there is no local-estimate fallback. `None` (no usage-carrying
/// round yet) renders as an em-dash placeholder.
mod resolve_ctx_used {
    use crate::render::resolve_ctx_used;

    #[test]
    fn real_context_is_used_verbatim() {
        assert_eq!(resolve_ctx_used(Some(9_100)), Some(9_100));
    }

    #[test]
    fn no_estimate_fallback_without_real_data() {
        // Even with a non-zero local estimate available, no real usage
        // means no display value — the bar shows `—` instead.
        assert_eq!(resolve_ctx_used(None), None);
    }
}

/// Before the first usage-carrying round (fresh session), the status bar
/// shows `—` for the ctx used count and 0% for the threshold meter — never
/// a local chars/4 estimate.
#[test]
fn status_bar_without_provider_truth_shows_placeholder_and_zero_percent() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(f, f.area(), "act", false, "", 0, None, 80000, 200000, 0);
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        row.contains("\u{2014}"),
        "ctx used must render the em-dash placeholder; got: {row}"
    );
    assert!(
        row.contains("0%"),
        "thr must read 0% without provider truth; got: {row}"
    );
    assert!(
        row.contains("200K"),
        "ctx denominator still shows the model window; got: {row}"
    );
}
