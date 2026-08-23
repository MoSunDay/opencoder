//! Copy-mode wrap plan and wrap-aware terminal backend.
//!
//! Terminals only omit the newline when joining *wrapped* (DECAWM
//! auto-wrapped) rows during native selection copy. Ratatui's crossterm
//! backend positions every visual row with `MoveTo` (CSI `y;x H`), so in the
//! terminal's eyes every row is a "hard" row and terminal-native copy inserts
//! a newline at every soft-wrap boundary of a long line. This module makes
//! the terminal *know* about display-only wraps: while copy mode is active,
//! [`WrapAwareBackend::draw`] suppresses the `MoveTo` at boundaries that the
//! per-frame [`WrapPlan`] marks as continuation rows, so the terminal's own
//! auto-wrap engine marks the row wrapped and copy joins it without a
//! newline. Real (hard) line breaks still get a `MoveTo` and keep their
//! newline; outside copy mode every byte is delegated verbatim.

use std::any::Any;
use std::cell::RefCell;
use std::io::{self, Stdout, Write};
use std::rc::Rc;

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{
    Attribute as CAttribute, Color as CColor, Colors, Print, SetAttribute, SetBackgroundColor,
    SetColors, SetForegroundColor, SetUnderlineColor,
};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color as RColor, Modifier};
use ratatui::Terminal;

use crate::composer::VisualRow;

/// Per-frame copy-mode wrap state, shared between the renderer (fills it)
/// and [`WrapAwareBackend`] (consumes it during `draw`). `soft` is indexed
/// by absolute terminal row: `soft[y] == true` means the visual row `y` is
/// the continuation of a longer logical line (a display-only wrap), so the
/// backend may let the terminal auto-wrap into it instead of `MoveTo`.
#[derive(Default)]
pub(crate) struct WrapPlan {
    /// Whether the current frame renders in copy mode.
    pub active: bool,
    /// Terminal text width the wrap model was computed at.
    pub term_width: u16,
    /// Absolute-row soft-wrap flags for the current frame.
    pub soft: Vec<bool>,
}

impl WrapPlan {
    /// Overwrite `soft[row..row + flags.len()]`, growing the vector as
    /// needed. Views fill disjoint row ranges (body, composer); the frame
    /// start clears the plan first (see `render`).
    pub(crate) fn set_soft(&mut self, row: usize, flags: &[bool]) {
        if flags.is_empty() {
            return;
        }
        let end = row.saturating_add(flags.len());
        if self.soft.len() < end {
            self.soft.resize(end, false);
        }
        self.soft[row..end].copy_from_slice(flags);
    }
}

/// Soft flags from a cumulative wrapped-row table (`cum_rows[i]` = screen
/// rows consumed by lines `[0..i]` — the `CleanModel` layout). Screen rows
/// `scroll_y..scroll_y + count` are marked soft when they are not the first
/// row of their logical line; rows past the last content row (blank tail of
/// the viewport) are hard.
pub(crate) fn soft_flags_from_cum_rows(
    cum_rows: &[usize],
    scroll_y: usize,
    count: usize,
) -> Vec<bool> {
    let total = cum_rows.last().copied().unwrap_or(0);
    (0..count)
        .map(|s| {
            let g = scroll_y.saturating_add(s);
            if g >= total {
                return false;
            }
            let line = cum_rows[1..].partition_point(|&r| r <= g);
            g != cum_rows[line]
        })
        .collect()
}

/// Soft flags from `composer::wrap_rows` output: a row continues the
/// previous one exactly when the previous row ends where this one starts (a
/// hard `'\n'` leaves a one-char gap for the newline).
pub(crate) fn soft_flags_from_wrap_rows(rows: &[VisualRow]) -> Vec<bool> {
    (0..rows.len())
        .map(|r| r > 0 && rows[r - 1].end == rows[r].start)
        .collect()
}

/// Soft flags from `notepad::editor::row_texts` output: a row continues the
/// previous one exactly when the previous row carries no trailing hard
/// newline.
pub(crate) fn soft_flags_from_row_texts(rows: &[String]) -> Vec<bool> {
    (0..rows.len())
        .map(|r| r > 0 && !rows[r - 1].ends_with('\n'))
        .collect()
}

/// [`Backend`] wrapper whose `draw` suppresses the `MoveTo` at soft-wrap
/// boundaries while copy mode is active, so the terminal's own DECAWM
/// auto-wrap marks those rows wrapped and native selection copy joins them
/// without newlines. Every other method — and the whole `draw` outside copy
/// mode — is delegated verbatim, so non-copy rendering is byte-identical to
/// the plain [`CrosstermBackend`].
pub(crate) struct WrapAwareBackend<W: Write> {
    inner: CrosstermBackend<W>,
    /// Shared with the renderer (see [`WrapPlan`]).
    pub plan: Rc<RefCell<WrapPlan>>,
}

impl<W: Write> WrapAwareBackend<W> {
    pub(crate) fn new(inner: CrosstermBackend<W>, plan: Rc<RefCell<WrapPlan>>) -> Self {
        Self { inner, plan }
    }
}

impl<W: Write> Write for WrapAwareBackend<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        // Disambiguated: `CrosstermBackend` has both `Backend::flush` and
        // `Write::flush`; the writer flush is what `Write` must delegate to.
        <CrosstermBackend<W> as Write>::flush(&mut self.inner)
    }
}

/// Mirror of ratatui's private `ModifierDiff::queue` (crossterm backend), so
/// the active-mode draw emits the same style transitions as the delegated
/// one.
fn queue_modifier_diff<W: Write>(w: &mut W, from: Modifier, to: Modifier) -> io::Result<()> {
    let removed = from - to;
    if removed.contains(Modifier::REVERSED) {
        queue!(w, SetAttribute(CAttribute::NoReverse))?;
    }
    if removed.contains(Modifier::BOLD) {
        queue!(w, SetAttribute(CAttribute::NormalIntensity))?;
        if to.contains(Modifier::DIM) {
            queue!(w, SetAttribute(CAttribute::Dim))?;
        }
    }
    if removed.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(CAttribute::NoItalic))?;
    }
    if removed.contains(Modifier::UNDERLINED) {
        queue!(w, SetAttribute(CAttribute::NoUnderline))?;
    }
    if removed.contains(Modifier::DIM) {
        queue!(w, SetAttribute(CAttribute::NormalIntensity))?;
    }
    if removed.contains(Modifier::CROSSED_OUT) {
        queue!(w, SetAttribute(CAttribute::NotCrossedOut))?;
    }
    if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
        queue!(w, SetAttribute(CAttribute::NoBlink))?;
    }
    let added = to - from;
    if added.contains(Modifier::REVERSED) {
        queue!(w, SetAttribute(CAttribute::Reverse))?;
    }
    if added.contains(Modifier::BOLD) {
        queue!(w, SetAttribute(CAttribute::Bold))?;
    }
    if added.contains(Modifier::ITALIC) {
        queue!(w, SetAttribute(CAttribute::Italic))?;
    }
    if added.contains(Modifier::UNDERLINED) {
        queue!(w, SetAttribute(CAttribute::Underlined))?;
    }
    if added.contains(Modifier::DIM) {
        queue!(w, SetAttribute(CAttribute::Dim))?;
    }
    if added.contains(Modifier::CROSSED_OUT) {
        queue!(w, SetAttribute(CAttribute::CrossedOut))?;
    }
    if added.contains(Modifier::SLOW_BLINK) {
        queue!(w, SetAttribute(CAttribute::SlowBlink))?;
    }
    if added.contains(Modifier::RAPID_BLINK) {
        queue!(w, SetAttribute(CAttribute::RapidBlink))?;
    }
    Ok(())
}

impl<W: Write> Backend for WrapAwareBackend<W> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let plan = self.plan.borrow();
        if !plan.active {
            drop(plan);
            // Non-copy frames: byte-identical to the plain backend.
            return self.inner.draw(content);
        }
        let width = plan.term_width;
        let soft = plan.soft.clone();
        drop(plan);
        let mut fg = RColor::Reset;
        let mut bg = RColor::Reset;
        let mut underline_color = RColor::Reset;
        let mut modifier = Modifier::empty();
        let mut last_pos: Option<Position> = None;
        // The terminal only enters its DECAWM wrap-pending state when a
        // visible symbol was printed at the last column; an empty symbol
        // (wide-char continuation cell) must not be treated as one.
        let mut last_printed = false;
        for (x, y, cell) in content {
            // Style sequences at the boundary are conservatively treated as
            // hard: some terminals cancel the pending wrap on SGR, so only
            // same-style continuations skip the MoveTo.
            let style_same = cell.modifier == modifier
                && cell.fg == fg
                && cell.bg == bg
                && cell.underline_color == underline_color;
            let soft_boundary = style_same
                && width > 0
                && x == 0
                && y > 0
                && last_printed
                && last_pos
                    == Some(Position {
                        x: width - 1,
                        y: y - 1,
                    })
                && soft.get(y as usize).copied().unwrap_or(false);
            if !soft_boundary && !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
                queue!(self, MoveTo(x, y))?;
            }
            last_pos = Some(Position { x, y });
            last_printed = !cell.symbol().is_empty();
            if cell.modifier != modifier {
                queue_modifier_diff(self, modifier, cell.modifier)?;
                modifier = cell.modifier;
            }
            if cell.fg != fg || cell.bg != bg {
                queue!(self, SetColors(Colors::new(cell.fg.into(), cell.bg.into())))?;
                fg = cell.fg;
                bg = cell.bg;
            }
            if cell.underline_color != underline_color {
                queue!(self, SetUnderlineColor(cell.underline_color.into()))?;
                underline_color = cell.underline_color;
            }
            queue!(self, Print(cell.symbol()))?;
        }
        queue!(
            self,
            SetForegroundColor(CColor::Reset),
            SetBackgroundColor(CColor::Reset),
            SetUnderlineColor(CColor::Reset),
            SetAttribute(CAttribute::Reset),
        )
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }
    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }
    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        <CrosstermBackend<W> as Write>::flush(&mut self.inner)
    }
}

#[cfg(test)]
#[path = "copy_wrap_fill_tests.rs"]
mod fill_tests;
#[cfg(test)]
#[path = "copy_wrap_tests.rs"]
mod tests;

/// Frame-start wrap-plan setup: returns `None` for non-production backends
/// (tests), mirrors `copy_mode` into `active`, and clears the previous
/// frame's soft flags so renderers re-fill them fresh.
pub(crate) fn frame_plan<B: Backend + 'static>(
    terminal: &mut Terminal<B>,
    copy_mode: bool,
) -> Option<Rc<RefCell<WrapPlan>>> {
    let plan: Option<Rc<RefCell<WrapPlan>>> = (terminal.backend() as &dyn Any)
        .downcast_ref::<WrapAwareBackend<Stdout>>()
        .map(|b| b.plan.clone());
    if let Some(plan) = &plan {
        let mut wp = plan.borrow_mut();
        wp.active = copy_mode;
        if copy_mode {
            wp.soft.clear();
        }
    }
    plan
}
