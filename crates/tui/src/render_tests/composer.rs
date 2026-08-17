use super::*;

/// The composer renders a `❯ ` prompt on the first line, the first input
/// segment after it, subsequent lines without a prompt, and a follow label
/// on the top border row.
#[test]
fn composer_renders_prompt_and_multiline_text() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_composer(
                f,
                Rect::new(0, 0, 40, 5),
                "hello\nworld",
                false, // copy_mode
                0,
                38,  // inner_w: 40 - 2 borders
                2,   // prompt_w: "❯ "
                &[], // no pending images
                false,
                None,
                None,
                &Line::raw("ignored"),
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    // Prompt glyph lands at the first inner cell (border=1).
    assert_eq!(buf[(1, 1)].symbol(), "\u{276f}", "prompt glyph at (1,1)");
    let row1 = row_text(buf, 1, 40);
    let row2 = row_text(buf, 2, 40);
    assert!(
        row1.contains('\u{276f}'),
        "prompt should appear on row 1; got: {row1}"
    );
    assert!(
        row1.contains("hello"),
        "hello should appear on row 1; got: {row1}"
    );
    assert!(
        row2.contains("world"),
        "world should appear on row 2; got: {row2}"
    );
}

/// The `/annotation` editor (`plan_mode` active, `edit_title == "edit
/// annotation"`) mirrors the body top-title (`workdir · model · effort`) on
/// its top border, right-aligned and coloured green — alongside the left
/// ` edit annotation ` label.
#[test]
fn annotation_editor_shows_green_top_title() {
    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    let green = crate::theme::ok_color();
    let top_title = Line::from(vec![
        Span::raw("/root/proj"),
        Span::raw(" \u{00b7} "),
        Span::raw("glm-5.2"),
        Span::raw(" \u{00b7} "),
        Span::raw("high"),
    ]);
    terminal
        .draw(|f| {
            render_composer(
                f,
                Rect::new(0, 0, 80, 6),
                "",
                false, // copy_mode
                0,
                78, // inner_w
                2,  // prompt_w
                &[],
                false,
                Some("PLAN"),
                Some("edit annotation"),
                &top_title,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let top = row_text(buf, 0, 80);
    // Left label present.
    assert!(top.contains("edit annotation"), "left label; got: {top}");
    // Right-aligned info title present on the same top border row.
    assert!(top.contains("glm-5.2"), "model in info title; got: {top}");
    // The model text must be green. Locate its cell x via char offset.
    let pos = top.find("glm-5.2").expect("model substring present");
    let cell_x = top[..pos].chars().count() as u16;
    let cell = buf.cell((cell_x, 0)).expect("model cell on top border row");
    assert_eq!(
        cell.style().fg,
        Some(green),
        "model must be green (annotation accent); got: {:?}; row: {top}",
        cell.style().fg
    );
}

/// The `/plan` editor must NOT receive the right-aligned info title, even when
/// a body top-title is in scope.
#[test]
fn plan_editor_has_no_info_top_title() {
    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    let top_title = Line::from(vec![
        Span::raw("/root/proj"),
        Span::raw(" \u{00b7} "),
        Span::raw("glm-5.2"),
    ]);
    terminal
        .draw(|f| {
            render_composer(
                f,
                Rect::new(0, 0, 80, 6),
                "",
                false, // copy_mode
                0,
                78,
                2,
                &[],
                false,
                Some("PLAN"),
                None, // edit_title None -> "edit plan", warn-coloured, NO info title
                &top_title,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let top = row_text(buf, 0, 80);
    assert!(top.contains("edit plan"), "plan label; got: {top}");
    assert!(
        !top.contains("glm-5.2"),
        "plan editor must not show info title; got: {top}"
    );
}

/// Copy mode integrates at the `render_composer` seam: with `copy_mode`
/// set, the function early-exits into the clean renderer — text flush at
/// column 0, no border, no prompt glyph — regardless of the other
/// decoration parameters (plan label, titles, badges).
#[test]
fn composer_copy_mode_param_early_exits_to_clean_view() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_composer(
                f,
                Rect::new(0, 0, 40, 6),
                "plain text",
                true, // copy_mode
                0,
                38,
                2,
                &[],
                false,
                Some("PLAN"),
                Some("edit annotation"),
                &Line::raw("ignored"),
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row0: String = (0..40)
        .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
        .collect();
    assert!(
        row0.starts_with("plain text"),
        "text flush at col 0: {row0:?}"
    );
    let all: String = (0..6)
        .flat_map(|y| (0..40).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            buf.cell((x, y))
                .unwrap()
                .symbol()
                .chars()
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(!all.contains('\u{276f}'), "no prompt glyph: {all:?}");
    assert!(
        !all.contains("edit annotation"),
        "decoration titles must not render in copy mode: {all:?}"
    );
    for deco in ['\u{250c}', '\u{2514}', '\u{2500}'] {
        assert!(
            !all.contains(deco),
            "border {deco:?} must be absent: {all:?}"
        );
    }
}
