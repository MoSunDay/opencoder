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
                0,
                38,  // inner_w: 40 - 2 borders
                2,   // prompt_w: "❯ "
                &[], // no pending images
                false,
                None,
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
