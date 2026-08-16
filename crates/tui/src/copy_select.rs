//! In-app copy/selection mode (default `Ctrl+G`): a vi-like cursor walks the
//! rendered body, `v` anchors a line selection, `y`/`Enter` yank it to the
//! system clipboard via OSC 52 ([`crate::osc52`]).
//!
//! Replaces the previous "hand the drag to the terminal" copy mode: the
//! selection now happens inside the app on every terminal (no Kitty/tmux
//! protocol dependency), and the yanked text is stripped of render
//! decoration (block headers, indent gutters, separators) so pastes are
//! clean. Wrapped rows are rejoined — a logical line spanning several screen
//! rows is copied as one line, never split at the wrap point.
//!
//! All state math (`next_state`/`move_target`/`ensure_visible`/`yank_text`/
//! `strip_decor`) is pure and unit-tested; only [`handle_key`] mutates the
//! caller's mode/scroll state, mirroring the old `copy_mode::handle_key`
//! contract (toggle flips, active mode swallows, inactive passes through —
//! except a toggle on an empty transcript, which yields [`CopyOutcome::Empty`]
//! for a status flash instead of entering the mode).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use ratatui::text::Line;

use crate::keymap::KeyBindings;
use crate::render_viewport::ViewportCache;
use crate::theme;

/// Anim-tick lifetime of the `COPIED` chip shown after a yank (10 FPS ticks).
pub const COPIED_FLASH_TICKS: u32 = 20;

/// Status-flash text shown when the copy toggle hits an empty transcript.
pub const EMPTY_FLASH_TEXT: &str = "empty \u{2014} nothing to copy";

/// In-app selection state. All rows are absolute content rows
/// (`screen row + scroll offset`) so the cursor/selection stay anchored to
/// the text while the viewport scrolls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopySel {
    /// Absolute row of the cursor.
    pub cursor: u32,
    /// Selection anchor row; `None` while just moving the cursor.
    pub anchor: Option<u32>,
    /// Anim tick of the last yank, for the transient `COPIED` chip.
    pub copied_at: Option<u32>,
}

impl CopySel {
    /// New state with the cursor on `top_row` (the viewport's top row).
    pub fn entry(top_row: u32) -> Self {
        CopySel {
            cursor: top_row,
            anchor: None,
            copied_at: None,
        }
    }

    /// Whether a selection is anchored.
    pub fn selecting(&self) -> bool {
        self.anchor.is_some()
    }

    /// Normalized inclusive row range `(lo, hi)` of the active selection.
    pub fn row_range(&self) -> Option<(usize, usize)> {
        self.anchor.map(|a| {
            let (a, c) = (a as usize, self.cursor as usize);
            (a.min(c), a.max(c))
        })
    }

    /// Whether the `COPIED` chip is still visible at `now_tick`. Uses
    /// wrapping subtraction so it stays correct across `anim_tick` wraparound.
    pub fn flash_active(&self, now_tick: u32) -> bool {
        self.copied_at
            .is_some_and(|s| now_tick.wrapping_sub(s) < COPIED_FLASH_TICKS)
    }

    /// Status-chip text for the current phase (`now_tick` for flash expiry).
    pub fn chip_text(&self, now_tick: u32) -> String {
        if self.flash_active(now_tick) {
            "COPIED (OSC52)".to_string()
        } else {
            "COPY: \u{2191}\u{2193}\u{2190}\u{2192} move \u{b7} v select \u{b7} y/Enter copy \u{b7} Esc exit".to_string()
        }
    }
}

/// Whether copy interactions currently suppress mouse handling: the explicit
/// in-app mode or the Shift-held native-selection path (`terminal.rs`).
pub fn is_active(sel: Option<&CopySel>, shift_held: bool) -> bool {
    sel.is_some() || shift_held
}

/// What the event loop should do after [`handle_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyOutcome {
    /// Key not consumed (mode inactive, not the toggle key) — pass through.
    Ignored,
    /// Consumed; stay in the mode.
    Consumed,
    /// Consumed; yank the selection now, keep the mode (with `COPIED` flash).
    Yank,
    /// Consumed; yank then exit the mode (Enter semantics).
    YankExit,
    /// Consumed; exit without copying (`Esc`/`q`/toggle).
    Exit,
    /// Consumed; the toggle hit an empty transcript (nothing to select) —
    /// the caller flashes feedback instead of entering the mode.
    Empty,
}

/// `true` for `c` typed with no modifiers (SHIFT tolerated for letters).
fn plain_char(k: &KeyEvent, c: char) -> bool {
    k.code == KeyCode::Char(c)
        && matches!(k.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
}

/// First absolute row occupied by the logical line containing `row`.
fn line_first_row(viewport: Option<&ViewportCache>, row: u32) -> u32 {
    match viewport {
        Some(v) if v.total_rows() > 0 => {
            v.row_of_line(v.line_at_row(row as usize)) as u32
        }
        _ => row,
    }
}

/// Last absolute row (inclusive) occupied by the logical line at `row`.
fn line_last_row(viewport: Option<&ViewportCache>, row: u32) -> u32 {
    match viewport {
        Some(v) if v.total_rows() > 0 => {
            let li = v.line_at_row(row as usize);
            (v.row_of_line(li + 1).saturating_sub(1)) as u32
        }
        _ => row,
    }
}

/// Target absolute row for a movement key from `cursor`. `None` for
/// non-movement keys. Clamps to `[0, total_rows-1]`. Pure.
fn move_target(
    k: &KeyEvent,
    cursor: u32,
    viewport: Option<&ViewportCache>,
    content_h: usize,
    total_rows: usize,
) -> Option<u32> {
    if total_rows == 0 {
        return None;
    }
    let max = (total_rows - 1) as u32;
    let page = content_h.max(1) as u32;
    let bare = k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT;
    let target = match k.code {
        KeyCode::Up if bare => cursor.saturating_sub(1),
        KeyCode::Down if bare => (cursor + 1).min(max),
        KeyCode::PageUp if bare => cursor.saturating_sub(page),
        KeyCode::PageDown if bare => (cursor + page).min(max),
        KeyCode::Home if bare => 0,
        KeyCode::End if bare => max,
        _ if plain_char(k, 'k') => cursor.saturating_sub(1),
        _ if plain_char(k, 'j') => (cursor + 1).min(max),
        _ if plain_char(k, 'h') => line_first_row(viewport, cursor),
        _ if plain_char(k, 'l') => line_last_row(viewport, cursor),
        _ if plain_char(k, 'g') => 0,
        _ if plain_char(k, 'G') => max,
        _ => return None,
    };
    Some(target)
}

/// Adjust `scroll` so absolute row `cursor` is inside the viewport
/// (`content_h` visible rows), clearing `follow` when the scroll moves —
/// cursor navigation takes over the viewport. Clamps to the scroll maximum.
/// Pure state math (mutates only through the `&mut` params).
pub fn ensure_visible(
    cursor: u32,
    scroll: &mut u32,
    content_h: usize,
    total_rows: usize,
    follow: &mut bool,
) {
    if content_h == 0 || total_rows == 0 {
        return;
    }
    let cursor = (cursor as usize).min(total_rows - 1);
    let cur = *scroll as usize;
    let target = if cursor < cur {
        cursor
    } else if cursor >= cur + content_h {
        cursor + 1 - content_h
    } else {
        cur
    };
    let clamped = target.min(total_rows.saturating_sub(content_h));
    if clamped != cur {
        *scroll = clamped as u32;
        *follow = false;
    }
}

/// Full key routing for copy/selection mode. Mirrors the old
/// `copy_mode::handle_key` contract (toggle key flips the mode, the active
/// mode swallows every key, inactive mode passes non-toggle keys through)
/// extended with cursor movement and selection anchoring. When the toggle
/// hits an empty transcript it is still consumed, but the caller receives
/// [`CopyOutcome::Empty`] to flash feedback instead of entering the mode.
///
/// Entering the mode parks the cursor on the current scroll top so the
/// visible window is unchanged. Returns [`CopyOutcome::Ignored`] when the
/// key belongs to the rest of the app.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_key(
    k: &KeyEvent,
    sel: &mut Option<CopySel>,
    keymap: &KeyBindings,
    viewport: Option<&ViewportCache>,
    content_h: usize,
    scroll: &mut u32,
    follow: &mut bool,
) -> CopyOutcome {
    let total_rows = viewport.map_or(0, |v| v.total_rows());
    let Some(s) = sel.as_mut() else {
        // Inactive: the toggle key enters the mode — or, on an empty
        // transcript (nothing to select), is still consumed but reports
        // [`CopyOutcome::Empty`] so the caller can flash feedback.
        if keymap.copy_mode.matches(k) {
            if total_rows > 0 {
                *sel = Some(CopySel::entry(*scroll));
                return CopyOutcome::Consumed;
            }
            return CopyOutcome::Empty;
        }
        return CopyOutcome::Ignored;
    };
    // Active: the mode owns the keyboard — every key is consumed.
    if keymap.copy_mode.matches(k) || k.code == KeyCode::Esc || plain_char(k, 'q') {
        return CopyOutcome::Exit;
    }
    if plain_char(k, 'v')
        || (k.code == KeyCode::Char(' ') && k.modifiers == KeyModifiers::NONE)
    {
        // Toggle the anchor: pressing `v` again drops the selection.
        s.anchor = if s.selecting() { None } else { Some(s.cursor) };
        return CopyOutcome::Consumed;
    }
    if k.code == KeyCode::Enter && k.modifiers == KeyModifiers::NONE {
        return CopyOutcome::YankExit;
    }
    if plain_char(k, 'y') {
        return CopyOutcome::Yank;
    }
    if let Some(target) = move_target(k, s.cursor, viewport, content_h, total_rows) {
        s.cursor = target;
        ensure_visible(target, scroll, content_h, total_rows, follow);
    }
    CopyOutcome::Consumed
}

/// Concatenate a rendered line's span contents into its plain text.
pub fn plain_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Strip render decoration from one flattened body line, returning `None`
/// for pure-decoration lines that must not appear in a copy:
/// - `❯ User:` / `❯ Say:` block headers,
/// - horizontal separators (runs of `─ ━ ═ ┅ -`),
/// - the 4-space content gutter the body renderer indents content with.
pub fn strip_decor(text: &str) -> Option<String> {
    let t = text.strip_prefix("    ").unwrap_or(text).trim_end();
    if t == "\u{276f} User:" || t == "\u{276f} Say:" {
        return None;
    }
    if is_separator(t) {
        return None;
    }
    Some(t.to_string())
}

/// `true` for a non-empty run (≥3) of a single horizontal-rule character.
fn is_separator(t: &str) -> bool {
    let mut chars = t.chars();
    match chars.next() {
        Some(c @ ('─' | '━' | '═' | '┅' | '-')) => {
            t.chars().count() >= 3 && t.chars().all(|x| x == c)
        }
        _ => false,
    }
}

/// Extract the yank text for `sel` from the viewport: the whole logical
/// lines covering the selection row range (a selection with no anchor yanks
/// the cursor's line), with wrapped rows rejoined into one line and render
/// decoration stripped. `None` when there is nothing to copy.
pub fn yank_text(viewport: Option<&ViewportCache>, sel: &CopySel) -> Option<String> {
    let v = viewport?;
    if v.total_rows() == 0 {
        return None;
    }
    let (a, b) = sel
        .row_range()
        .unwrap_or((sel.cursor as usize, sel.cursor as usize));
    let mut out = Vec::new();
    for li in v.line_at_row(a)..=v.line_at_row(b) {
        if let Some(line) = v.lines().get(li) {
            if let Some(text) = strip_decor(&plain_text(line)) {
                out.push(text);
            }
        }
    }
    let joined = out.join("\n");
    (!joined.is_empty()).then_some(joined)
}

/// Apply selection/cursor styling to the visible `lines` slice (`lines[i]`
/// is logical line `start + i` of `cache`). Lines whose row span intersects
/// the active selection get the selection background; with no selection the
/// cursor's line is underlined so the cursor stays visible. Mutates span
/// styles in place.
pub fn highlight_lines(lines: &mut [Line<'static>], cache: &ViewportCache, start: usize, sel: &CopySel) {
    let range = sel.row_range();
    for (i, line) in lines.iter_mut().enumerate() {
        let li = start + i;
        let row_lo = cache.row_of_line(li);
        let row_hi = cache.row_of_line(li + 1); // exclusive
        let selected = range.is_some_and(|(a, b)| row_lo <= b && row_hi > a);
        let cursor_line = range.is_none() && (row_lo..row_hi).contains(&(sel.cursor as usize));
        for span in line.spans.iter_mut() {
            if selected {
                span.style = span.style.bg(theme::highlight_bg());
            } else if cursor_line {
                span.style = span.style.add_modifier(Modifier::UNDERLINED);
            }
        }
    }
}

/// Post-process a non-`Ignored` [`CopyOutcome`] from [`handle_key`]:
/// perform the OSC 52 yank for `Yank`/`YankExit` (stamping the `COPIED`
/// flash), and clear the mode on the exit outcomes. Keeps the app loop's
/// call site a thin two-liner.
pub(crate) fn apply_key(
    outcome: CopyOutcome,
    sel: &mut Option<CopySel>,
    viewport: Option<&ViewportCache>,
    anim_tick: u32,
) {
    if matches!(outcome, CopyOutcome::Yank | CopyOutcome::YankExit) {
        if let Some(text) = sel.as_ref().and_then(|s| yank_text(viewport, s)) {
            crate::osc52::copy(&text);
            if let Some(s) = sel.as_mut() {
                s.copied_at = Some(anim_tick);
            }
        }
    }
    if matches!(outcome, CopyOutcome::YankExit | CopyOutcome::Exit) {
        *sel = None;
    }
}

/// Full copy-mode key dispatch for the app loop: route `k` through
/// [`handle_key`], apply its effects ([`apply_key`]: OSC 52 yank, mode
/// clear) and set the transient status flash on an empty-transcript toggle.
/// Returns `true` when the key was consumed (`false` ⇒ pass through to the
/// rest of the app); `mode_flash` mirrors the loop's status-chip state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_key(
    k: &KeyEvent,
    sel: &mut Option<CopySel>,
    keymap: &KeyBindings,
    viewport: Option<&ViewportCache>,
    content_h: usize,
    scroll: &mut u32,
    follow: &mut bool,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
) -> bool {
    let outcome = handle_key(k, sel, keymap, viewport, content_h, scroll, follow);
    if outcome == CopyOutcome::Ignored {
        return false;
    }
    apply_key(outcome, sel, viewport, anim_tick);
    if outcome == CopyOutcome::Empty {
        *mode_flash = Some((EMPTY_FLASH_TEXT.to_string(), anim_tick));
    }
    true
}

#[cfg(test)]
#[path = "copy_select_tests.rs"]
mod tests;
