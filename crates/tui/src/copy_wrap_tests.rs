//! Tests for `copy_wrap`: byte-level backend assertions against a capturing
//! writer, pure soft-flag functions, and plan-fill integration through the
//! three copy-mode clean renderers.

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Cell;
use ratatui::style::Style;

use crate::copy_wrap::*;
use crate::copy_wrap::{WrapAwareBackend, WrapPlan};

/// Capture writer: the reference and wrapped backends write into the same
/// shared buffer so outputs can be compared byte-for-byte.
#[derive(Clone, Default)]
struct Capture(Rc<RefCell<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Capture, Rc<RefCell<Vec<u8>>>) {
    let buf = Rc::new(RefCell::new(Vec::new()));
    (Capture(buf.clone()), buf)
}

/// Run a (x, y, symbol) cell stream through `backend.draw` + `flush`.
fn run<B: Backend>(backend: &mut B, cells: &[(u16, u16, &str)]) {
    let owned: Vec<(u16, u16, Cell)> = cells
        .iter()
        .map(|(x, y, s)| {
            let mut cell = Cell::default();
            cell.set_symbol(s);
            (*x, *y, cell)
        })
        .collect();
    backend
        .draw(owned.iter().map(|(x, y, c)| (*x, *y, c)))
        .unwrap();
    backend.flush().unwrap();
}

fn out(buf: &Rc<RefCell<Vec<u8>>>) -> String {
    String::from_utf8(buf.borrow().clone()).unwrap()
}

type Harness = (
    WrapAwareBackend<Capture>,
    Rc<RefCell<Vec<u8>>>,
    Rc<RefCell<WrapPlan>>,
);

/// Wrapped backend with `active` and `soft` flags preset.
fn wrapped(active: bool, width: u16, soft: &[bool]) -> Harness {
    let (cap, buf) = capture();
    let plan = Rc::new(RefCell::new(WrapPlan {
        active,
        term_width: width,
        soft: soft.to_vec(),
    }));
    let backend = WrapAwareBackend::new(CrosstermBackend::new(cap), plan.clone());
    (backend, buf, plan)
}

fn plain() -> (CrosstermBackend<Capture>, Rc<RefCell<Vec<u8>>>) {
    let (cap, buf) = capture();
    (CrosstermBackend::new(cap), buf)
}

/// One full 5-column row followed by a partial second row: the classic
/// soft-wrap boundary at (4,0) -> (0,1).
fn wrap_cells() -> Vec<(u16, u16, &'static str)> {
    vec![
        (0, 0, "a"),
        (1, 0, "b"),
        (2, 0, "c"),
        (3, 0, "d"),
        (4, 0, "e"),
        (0, 1, "f"),
        (1, 1, "g"),
        (2, 1, "h"),
    ]
}

#[test]
fn inactive_output_matches_plain_backend_byte_for_byte() {
    let cells = wrap_cells();
    let (mut b, buf) = plain();
    run(&mut b, &cells);
    let expected = out(&buf);

    let (mut b, buf, _plan) = wrapped(false, 5, &[false, false]);
    run(&mut b, &cells);
    assert_eq!(out(&buf), expected, "inactive wrapper must not alter bytes");
}

#[test]
fn soft_boundary_skips_moveto_and_relies_on_terminal_wrap() {
    let cells = wrap_cells();
    let (mut b, buf, _plan) = wrapped(true, 5, &[false, true, false]);
    run(&mut b, &cells);
    let got = out(&buf);
    // The wrapped stream must be byte-identical to the current crossterm
    // backend except for the one intentionally suppressed row move. Deriving
    // the style-reset suffix keeps this contract stable across equivalent
    // crossterm encodings (`CSI m` versus separate 39/49/59 resets).
    let (mut plain_backend, plain_buf) = plain();
    run(&mut plain_backend, &cells);
    let expected = out(&plain_buf).replacen("\x1b[2;1H", "", 1);
    assert_eq!(got, expected);
    assert!(
        !got.contains("\x1b[2;1H"),
        "soft boundary must skip MoveTo: {got:?}"
    );
}

#[test]
fn hard_boundary_keeps_moveto_identical_to_plain() {
    let cells = wrap_cells();
    let (mut b, buf, _plan) = wrapped(true, 5, &[false, false, false]);
    run(&mut b, &cells);
    let got = out(&buf);
    assert!(
        got.contains("\x1b[2;1H"),
        "hard boundary needs MoveTo: {got:?}"
    );
    let (mut pb, pbuf) = plain();
    run(&mut pb, &cells);
    assert_eq!(got, out(&pbuf));
}

#[test]
fn exact_width_line_joins_inside_but_stays_hard_at_next_line() {
    // Logical line "abcdef" (2×5) renders rows 0+1 (soft), then a hard line
    // break before row 2: the (4,0)->(0,1) jump must skip MoveTo, while the
    // (4,1)->(0,2) jump — the real newline — must keep it.
    let cells = vec![
        (0, 0, "a"),
        (1, 0, "b"),
        (2, 0, "c"),
        (3, 0, "d"),
        (4, 0, "e"),
        (0, 1, "f"),
        (1, 1, "g"),
        (2, 1, "h"),
        (3, 1, "i"),
        (4, 1, "j"),
        (0, 2, "k"),
    ];
    let (mut b, buf, _plan) = wrapped(true, 5, &[false, true, false]);
    run(&mut b, &cells);
    let got = out(&buf);
    assert!(
        !got.contains("\x1b[2;1H"),
        "soft continuation must skip MoveTo: {got:?}"
    );
    assert!(
        got.contains("\x1b[3;1H"),
        "real newline must keep MoveTo: {got:?}"
    );
}

#[test]
fn style_change_at_boundary_falls_back_to_moveto() {
    let mut cells = vec![
        (0, 0, "a"),
        (1, 0, "b"),
        (2, 0, "c"),
        (3, 0, "d"),
        (4, 0, "e"),
    ];
    let mut styled = Cell::default();
    styled.set_symbol("f");
    styled.set_style(Style::default().fg(ratatui::style::Color::Red));
    cells.push((0, 1, "f"));
    let (mut b, buf, _plan) = wrapped(true, 5, &[false, true]);
    let owned: Vec<(u16, u16, Cell)> = cells
        .iter()
        .map(|(x, y, s)| {
            let mut cell = Cell::default();
            cell.set_symbol(s);
            if (*x, *y) == (0, 1) {
                cell = styled.clone();
            }
            (*x, *y, cell)
        })
        .collect();
    b.draw(owned.iter().map(|(x, y, c)| (*x, *y, c))).unwrap();
    <WrapAwareBackend<Capture> as Backend>::flush(&mut b).unwrap();
    assert!(
        out(&buf).contains("\x1b[2;1H"),
        "style change at the boundary must fall back to MoveTo: {:?}",
        out(&buf)
    );
}

#[test]
fn empty_symbol_at_last_column_falls_back_to_moveto() {
    // A wide-char continuation cell has an empty symbol: it prints nothing,
    // so no DECAWM wrap-pending state exists — must not skip the MoveTo.
    let cells = vec![
        (0, 0, "a"),
        (1, 0, "b"),
        (2, 0, "c"),
        (3, 0, "d"),
        (4, 0, ""),
        (0, 1, "f"),
    ];
    let (mut b, buf, _plan) = wrapped(true, 5, &[false, true]);
    run(&mut b, &cells);
    assert!(
        out(&buf).contains("\x1b[2;1H"),
        "empty last-column symbol: {:?}",
        out(&buf)
    );
}

#[test]
fn jump_not_from_last_column_is_hard_even_when_soft() {
    // Row 0 only reaches column 3: the (3,0)->(0,1) jump is not a wrap
    // boundary, so it must be a MoveTo regardless of the soft flag.
    let cells = vec![
        (0, 0, "a"),
        (1, 0, "b"),
        (2, 0, "c"),
        (3, 0, "d"),
        (0, 1, "f"),
    ];
    let (mut b, buf, _plan) = wrapped(true, 5, &[false, true]);
    run(&mut b, &cells);
    assert!(
        out(&buf).contains("\x1b[2;1H"),
        "non-boundary jump: {:?}",
        out(&buf)
    );
}

#[test]
fn zero_width_and_missing_soft_flags_are_hard() {
    let cells = wrap_cells();
    let (mut b, buf, _plan) = wrapped(true, 0, &[false, true]);
    run(&mut b, &cells);
    assert!(
        out(&buf).contains("\x1b[2;1H"),
        "width 0 must keep MoveTo: {:?}",
        out(&buf)
    );
    let (mut b, buf, _plan) = wrapped(true, 5, &[]);
    run(&mut b, &cells);
    assert!(
        out(&buf).contains("\x1b[2;1H"),
        "out-of-range soft must keep MoveTo: {:?}",
        out(&buf)
    );
}

#[test]
fn set_soft_splices_and_grows() {
    let mut plan = WrapPlan::default();
    plan.set_soft(0, &[false, true, true]);
    assert_eq!(plan.soft, vec![false, true, true]);
    // Body then composer fill disjoint ranges; frame start clears first.
    plan.set_soft(10, &[false, true]);
    assert_eq!(plan.soft.len(), 12);
    assert!(!plan.soft[9]);
    assert!(!plan.soft[10] && plan.soft[11]);
    plan.set_soft(0, &[false]);
    assert!(!plan.soft[0], "splice must overwrite, not append");
}

// ── Pure soft-flag functions ────────────────────────────────────────────

#[test]
fn cum_rows_flags() {
    // One 3-row logical line: first row hard, continuations soft.
    assert_eq!(
        soft_flags_from_cum_rows(&[0, 3], 0, 3),
        vec![false, true, true]
    );
    // Scrolled into the line: every visible row is a continuation.
    assert_eq!(soft_flags_from_cum_rows(&[0, 3], 1, 2), vec![true, true]);
    // Short lines: all hard.
    assert_eq!(
        soft_flags_from_cum_rows(&[0, 1, 2], 0, 2),
        vec![false, false]
    );
    // Exact-multiple line (2×width) then a new line at the row boundary.
    assert_eq!(
        soft_flags_from_cum_rows(&[0, 2, 4], 0, 4),
        vec![false, true, false, true]
    );
    // Blank viewport tail past the content is hard (no phantom joins).
    assert_eq!(
        soft_flags_from_cum_rows(&[0, 2], 0, 5),
        vec![false, true, false, false, false]
    );
    // Empty model.
    assert_eq!(
        soft_flags_from_cum_rows(&[0], 0, 3),
        vec![false, false, false]
    );
    // Empty line between content lines is its own hard row.
    assert_eq!(
        soft_flags_from_cum_rows(&[0, 1, 2], 0, 3),
        vec![false, false, false]
    );
}

#[test]
fn wrap_rows_flags() {
    let rows = |s: &str, w: u16| crate::composer::wrap_rows(s, w, 0);
    assert_eq!(
        soft_flags_from_wrap_rows(&rows("abcdef", 3)),
        vec![false, true]
    );
    assert_eq!(
        soft_flags_from_wrap_rows(&rows("hello\nworld", 3)),
        vec![false, true, false, true]
    );
    assert_eq!(
        soft_flags_from_wrap_rows(&rows("ab cd", 4)),
        vec![false, true]
    );
    assert_eq!(
        soft_flags_from_wrap_rows(&rows("a\n", 3)),
        vec![false, false]
    );
    assert_eq!(soft_flags_from_wrap_rows(&rows("", 3)), vec![false]);
    assert_eq!(soft_flags_from_wrap_rows(&rows("abc", 3)), vec![false]);
}

#[test]
fn row_texts_flags() {
    let r = |s: &str, w: u16| crate::notepad::editor::row_texts(s, w);
    assert_eq!(
        soft_flags_from_row_texts(&r("alpha beta gamma\ndelta", 6)),
        vec![false, true, true, false]
    );
    assert_eq!(
        soft_flags_from_row_texts(&r("abc\ndef", 3)),
        vec![false, false]
    );
    assert_eq!(
        soft_flags_from_row_texts(&r("abc\n", 3)),
        vec![false, false]
    );
    assert_eq!(soft_flags_from_row_texts(&r("x", 3)), vec![false]);
}
