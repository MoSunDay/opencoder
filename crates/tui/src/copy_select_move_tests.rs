//! Tests for [`crate::copy_select`] movement, wrapped-line rejoin yanks,
//! decoration stripping, chip phases and highlight styling.

use super::super::*;
use crossterm::event::{KeyCode, KeyModifiers};
use opencoder_core::Config;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::chat::ChatView;
use crate::keymap::KeyBindings;

fn keybindings() -> KeyBindings {
    KeyBindings::from_config(&Config::default())
}

fn plain(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A view whose flattened lines are exactly `lines` (one Marker block per
/// line) — markers render verbatim, independent of the markdown renderer.
fn view_from_lines(lines: &[&str]) -> ChatView {
    let mut v = ChatView::default();
    for &l in lines {
        v.push_marker(Line::from(l.to_string()));
    }
    v
}

/// Viewport over `lines` word-wrapped at `width`.
fn cache(lines: &[&str], width: u16) -> ViewportCache {
    ViewportCache::build(&view_from_lines(lines), width, 0, 0)
}

// ── movement ────────────────────────────────────────────────────────────────

#[test]
fn arrows_and_hjkl_move_and_clamp() {
    let kb = keybindings();
    let c = cache(&["a", "b", "c"], 40); // 3 rows
    let mut sel = Some(CopySel::entry(1));
    let (mut scroll, mut follow) = (0u32, false);
    let mut step = |sel: &mut Option<CopySel>, k: KeyEvent| {
        handle_key(&k, sel, &kb, Some(&c), 5, &mut scroll, &mut follow)
    };
    step(&mut sel, key(KeyCode::Down));
    assert_eq!(sel.as_ref().unwrap().cursor, 2);
    step(&mut sel, key(KeyCode::Down));
    assert_eq!(sel.as_ref().unwrap().cursor, 2, "clamped at the last row");
    step(&mut sel, key(KeyCode::Up));
    step(&mut sel, plain('k'));
    assert_eq!(sel.as_ref().unwrap().cursor, 0);
    step(&mut sel, key(KeyCode::Up));
    assert_eq!(sel.as_ref().unwrap().cursor, 0, "clamped at the top");
    step(&mut sel, plain('j'));
    assert_eq!(sel.as_ref().unwrap().cursor, 1);
}

#[test]
fn page_home_end_jump() {
    let kb = keybindings();
    let lines: Vec<&str> = vec!["x"; 30];
    let c = cache(&lines, 40); // 30 rows
    let mut sel = Some(CopySel::entry(0));
    let (mut scroll, mut follow) = (0u32, false);
    let mut step = |sel: &mut Option<CopySel>, k: KeyEvent| {
        handle_key(&k, sel, &kb, Some(&c), 10, &mut scroll, &mut follow)
    };
    step(&mut sel, key(KeyCode::PageDown));
    assert_eq!(sel.as_ref().unwrap().cursor, 10);
    step(&mut sel, key(KeyCode::End));
    assert_eq!(sel.as_ref().unwrap().cursor, 29);
    step(&mut sel, key(KeyCode::Home));
    assert_eq!(sel.as_ref().unwrap().cursor, 0);
    step(&mut sel, key(KeyCode::PageUp));
    assert_eq!(sel.as_ref().unwrap().cursor, 0, "clamped at the top");
}

#[test]
fn h_and_l_jump_within_wrapped_line_rows() {
    let kb = keybindings();
    // One 20-char line at width 10 -> rows 0..1, plus a second line at row 2.
    let c = cache(&["0123456789abcdefghij", "b"], 10);
    assert_eq!(c.total_rows(), 3);
    let mut sel = Some(CopySel {
        cursor: 1,
        anchor: None,
        copied_at: None,
    });
    let (mut scroll, mut follow) = (0u32, false);
    // l -> last row of the wrapped line (1); h -> first row (0).
    handle_key(&plain('l'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow);
    assert_eq!(sel.as_ref().unwrap().cursor, 1);
    handle_key(&plain('h'), &mut sel, &kb, Some(&c), 5, &mut scroll, &mut follow);
    assert_eq!(sel.as_ref().unwrap().cursor, 0);
}

#[test]
fn ensure_visible_scrolls_cursor_into_view_and_clears_follow() {
    let (mut scroll, mut follow) = (10u32, true);
    ensure_visible(3, &mut scroll, 5, 40, &mut follow);
    assert_eq!(scroll, 3);
    assert!(!follow);
    // Below the window: scroll so the cursor is the last visible row.
    let (mut scroll, mut follow) = (0u32, true);
    ensure_visible(9, &mut scroll, 5, 40, &mut follow);
    assert_eq!(scroll, 5);
    // Already visible: no scroll change, follow untouched.
    let (mut scroll, mut follow) = (4u32, true);
    ensure_visible(6, &mut scroll, 5, 40, &mut follow);
    assert_eq!(scroll, 4);
    assert!(follow);
    // Scroll clamps to total-content.
    let (mut scroll, mut follow) = (0u32, true);
    ensure_visible(39, &mut scroll, 5, 40, &mut follow);
    assert_eq!(scroll, 35);
}

// ── strip_decor / yank_text ─────────────────────────────────────────────────

#[test]
fn strip_decor_drops_headers_separators_and_gutter() {
    assert_eq!(strip_decor("\u{276f} User:"), None);
    assert_eq!(strip_decor("\u{276f} Say:"), None);
    assert_eq!(strip_decor("──────────"), None);
    assert_eq!(strip_decor("----"), None);
    assert_eq!(strip_decor("    indented"), Some("indented".to_string()));
    assert_eq!(strip_decor("plain  "), Some("plain".to_string()));
    assert_eq!(strip_decor("    code  x"), Some("code  x".to_string()));
    // Short dash runs / single border chars are content, not separators.
    assert_eq!(strip_decor("--"), Some("--".to_string()));
    assert_eq!(strip_decor("a - b"), Some("a - b".to_string()));
}

#[test]
fn yank_without_selection_copies_the_cursor_line() {
    let c = cache(&["alpha", "beta"], 40);
    let sel = CopySel {
        cursor: 1,
        anchor: None,
        copied_at: None,
    };
    assert_eq!(yank_text(Some(&c), &sel).as_deref(), Some("beta"));
}

#[test]
fn yank_joins_selected_lines_with_newlines() {
    let c = cache(&["one", "two", "three"], 40);
    let sel = CopySel {
        cursor: 2,
        anchor: Some(0),
        copied_at: None,
    };
    assert_eq!(
        yank_text(Some(&c), &sel).as_deref(),
        Some("one\ntwo\nthree")
    );
    // Reversed anchor normalizes to the same text.
    let sel = CopySel {
        cursor: 0,
        anchor: Some(2),
        copied_at: None,
    };
    assert_eq!(
        yank_text(Some(&c), &sel).as_deref(),
        Some("one\ntwo\nthree")
    );
}

/// Regression: a logical line wrapping across several screen rows must be
/// yanked as ONE line — selecting only the wrapped continuation row still
/// copies the full logical text, with no newline at the wrap point.
#[test]
fn yank_rejoins_wrapped_rows_into_one_line() {
    // 25 chars at width 10 -> 3 screen rows.
    let text = "aaaa bbbb ccccc dddd";
    let c = cache(&[text], 10);
    assert!(c.total_rows() >= 2, "line must wrap in the fixture");
    for row in 0..c.total_rows() {
        let sel = CopySel {
            cursor: row as u32,
            anchor: None,
            copied_at: None,
        };
        assert_eq!(
            yank_text(Some(&c), &sel).as_deref(),
            Some(text),
            "row {row} belongs to the same logical line"
        );
    }
    // A selection spanning only rows 1..1 (the middle of the wrap) still
    // yields the single rejoined line.
    let sel = CopySel {
        cursor: 1,
        anchor: Some(1),
        copied_at: None,
    };
    let got = yank_text(Some(&c), &sel).expect("non-empty");
    assert_eq!(got, text);
    assert!(!got.contains('\n'), "wrap point must not inject a newline");
}

/// Regression: selection across two wrapped lines yanks exactly two lines.
#[test]
fn yank_two_wrapped_lines_rejoins_each() {
    let c = cache(&["aaaa bbbb ccc", "dddd eeee fff"], 8);
    let sel = CopySel {
        cursor: (c.total_rows() - 1) as u32,
        anchor: Some(0),
        copied_at: None,
    };
    assert_eq!(
        yank_text(Some(&c), &sel).as_deref(),
        Some("aaaa bbbb ccc\ndddd eeee fff")
    );
}

#[test]
fn yank_returns_none_for_empty_or_missing_viewport() {
    let sel = CopySel::entry(0);
    assert_eq!(yank_text(None, &sel), None);
    let empty = ViewportCache::build(&ChatView::default(), 40, 0, 0);
    assert_eq!(yank_text(Some(&empty), &sel), None);
}

#[test]
fn plain_text_concatenates_spans() {
    let line = Line::from(vec![Span::raw("ab"), Span::raw("cd")]);
    assert_eq!(plain_text(&line), "abcd");
    assert_eq!(plain_text(&Line::from("")), "");
}

// ── chip text / flash ───────────────────────────────────────────────────────

#[test]
fn chip_text_phases_between_hint_and_copied() {
    let mut s = CopySel::entry(0);
    assert!(s.chip_text(100).contains("COPY"));
    s.copied_at = Some(100);
    assert_eq!(s.chip_text(105), "COPIED (OSC52)");
    assert!(!s.flash_active(100 + COPIED_FLASH_TICKS), "flash expires");
    assert!(s.chip_text(100 + COPIED_FLASH_TICKS).contains("COPY"));
}

// ── highlight_lines ─────────────────────────────────────────────────────────

#[test]
fn highlight_lines_marks_selection_and_cursor() {
    let c = cache(&["aa", "bb", "cc"], 40);
    // Select rows 1..=1 (line 1); cursor sits on line 1.
    let sel = CopySel {
        cursor: 1,
        anchor: Some(1),
        copied_at: None,
    };
    let mut lines: Vec<Line<'static>> = vec![Line::from("aa"), Line::from("bb"), Line::from("cc")];
    highlight_lines(&mut lines, &c, 0, &sel);
    let bg = |l: &Line| l.spans[0].style.bg;
    assert_eq!(bg(&lines[0]), None);
    assert_eq!(bg(&lines[1]), Some(theme::highlight_bg()));
    assert_eq!(bg(&lines[2]), None);

    // No selection: the cursor's line is underlined instead.
    let sel = CopySel {
        cursor: 2,
        anchor: None,
        copied_at: None,
    };
    let mut lines: Vec<Line<'static>> = vec![Line::from("aa"), Line::from("bb"), Line::from("cc")];
    highlight_lines(&mut lines, &c, 0, &sel);
    assert!(!lines[0].spans[0].style.add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    assert!(!lines[1].spans[0].style.add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    assert!(lines[2].spans[0].style.add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    assert_eq!(lines[2].spans[0].style.bg, None);
    // Existing span styling survives (fg kept, bg added on selection).
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        "bb",
        Style::default().fg(Color::Red),
    ))];
    let sel = CopySel {
        cursor: 1,
        anchor: Some(1),
        copied_at: None,
    };
    highlight_lines(&mut lines, &c, 1, &sel);
    assert_eq!(lines[0].spans[0].style.fg, Some(Color::Red));
    assert_eq!(lines[0].spans[0].style.bg, Some(theme::highlight_bg()));
}
