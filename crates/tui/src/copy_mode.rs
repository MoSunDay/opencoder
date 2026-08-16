//! Copy mode (default `Ctrl+G`): hand text selection back to the terminal
//! emulator, and render the transcript undecorated so terminal-native copy
//! shortcuts yield clean text.
//!
//! Entering copy mode suspends the TUI's own mouse capture AND disables
//! tmux's `mouse` interception, handing raw mouse drags back to the terminal
//! emulator so its native text selection works on every terminal — not just
//! Kitty/WezTerm. On exit the TUI mouse capture is resumed, but tmux's
//! `mouse` is left off (see [`crate::tmux_mouse`]) to avoid re-introducing
//! the selection fight.
//!
//! While active, the body is re-rendered full-width with render decoration
//! stripped ([`clean_text`]): no rounded border, no scrollbar column, no
//! `[turn cost]` row, no border indicators; per row the indent gutter and
//! code-frame prefixes (`│ `, `▎ `) are removed and pure-decoration rows
//! (role headers `❯ User:`/`❯ Say:`, separators, `┌ lang`/`└───` code
//! frames) are dropped. The app itself never touches the clipboard —
//! copying is the terminal's own job via its native selection shortcuts.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::chat::ChatView;
use crate::keymap::KeyBindings;
use crate::render_viewport::ViewportCache;
use crate::terminal::{resume_mouse_capture, suspend_mouse_capture};

/// Enter copy mode: suspend our mouse capture and turn off tmux mouse
/// interception. Best-effort — terminal errors are ignored.
pub fn enter() {
    let _ = suspend_mouse_capture();
    // Discard previous state: we keep tmux mouse off on exit.
    let _ = crate::tmux_mouse::disable();
}

/// Exit copy mode: resume our mouse capture. tmux mouse is left off.
pub fn exit() {
    let _ = resume_mouse_capture();
}

/// Whether copy mode is currently suppressing mouse interactions. True when
/// the explicit toggle is on OR the user is holding Shift (the
/// Kitty-keyboard-protocol native-selection path tracked by `terminal.rs`).
pub fn is_active(copy_mode: bool, shift_held: bool) -> bool {
    copy_mode || shift_held
}

/// Pure decision logic for a key in the copy-mode context. No I/O — fully
/// unit-testable. Returns `(new_active, consumed)`:
/// - toggle key pressed -> flip `active`, consumed.
/// - active + any other key -> swallowed (consumed); `Esc` clears `active`.
/// - inactive + non-toggle key -> passed through (not consumed).
fn next_state(k: &KeyEvent, active: bool, keymap: &KeyBindings) -> (bool, bool) {
    if keymap.copy_mode.matches(k) {
        (!active, true)
    } else if active {
        let exiting = k.code == KeyCode::Esc;
        (active && !exiting, true)
    } else {
        (active, false)
    }
}

/// Handle a key for copy-mode toggle / input swallowing, performing the
/// enter/exit side effects. Returns `true` if the key was consumed (the
/// caller should mark the frame dirty and `continue`).
pub(crate) fn handle_key(k: &KeyEvent, copy_mode: &mut bool, keymap: &KeyBindings) -> bool {
    let prev = *copy_mode;
    let (next, consumed) = next_state(k, prev, keymap);
    if consumed {
        *copy_mode = next;
        if next && !prev {
            enter();
        } else if !next && prev {
            exit();
        }
    }
    consumed
}

// ── Clean-view layer ─────────────────────────────────────────────────────

/// Concatenate a rendered `Line`'s span contents into plain text.
pub fn plain_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// `true` when `t` consists solely of `≥ 3` copies of one rule character
/// (`─ ━ ═ ┅ -`) — a pure-decoration separator row.
fn is_separator(t: &str) -> bool {
    let mut chars = t.chars();
    match chars.next() {
        Some(c @ ('─' | '━' | '═' | '┅' | '-')) => {
            t.chars().count() >= 3 && t.chars().all(|x| x == c)
        }
        _ => false,
    }
}

/// Strip one render-decoration slot from `t`: first the fixed indent gutter
/// (4 spaces for user/assistant/image blocks, 2 for plan blocks), then the
/// code-frame `│ ` (or bare `│` on empty code rows) and blockquote `▎ `
/// prefixes. Deeper leading indentation beyond the fixed slot is preserved.
fn strip_slots(t: &str) -> &str {
    let t = t
        .strip_prefix("    ")
        .or_else(|| t.strip_prefix("  "))
        .unwrap_or(t);
    let t = t
        .strip_prefix("\u{2502} ")
        .or_else(|| t.strip_prefix("\u{2502}"))
        .unwrap_or(t);
    t.strip_prefix("\u{258e} ").unwrap_or(t)
}

/// Clean one rendered row for the copy-mode view. Returns `None` for
/// pure-decoration rows that are dropped entirely (role headers,
/// separators, code-frame borders); otherwise the row with its gutter /
/// prefix slots stripped. Semantic headers (`▸ tool`, `💭 Thinking`) and
/// any remaining indentation are kept — only decoration goes.
pub fn clean_text(text: &str) -> Option<String> {
    let t = strip_slots(text).trim_end();
    if t == "\u{276f} User:" || t == "\u{276f} Say:" {
        return None;
    }
    if t.starts_with('\u{250c}') || t.starts_with('\u{2514}') {
        return None; // `┌ lang` top / `└───` bottom code-frame borders
    }
    if is_separator(t) {
        return None;
    }
    Some(t.to_string())
}

/// Render the transcript for copy mode: full width, no block/border, no
/// scrollbar, no `[turn cost]` row, no border indicators — every visible
/// row cleaned via [`clean_text`] so terminal-native selection spans clean
/// text. Reuses the shared viewport cache; the width check naturally
/// rebuilds it when toggling in/out of copy mode (full width vs inner).
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_clean(
    f: &mut Frame,
    area: Rect,
    chat: &ChatView,
    scroll: &mut u32,
    follow: bool,
    anim_tick: u32,
    now_ms: i64,
    viewport: &mut Option<ViewportCache>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let needs_rebuild = viewport.as_ref().is_none_or(|v| v.width() != area.width);
    if needs_rebuild {
        *viewport = Some(ViewportCache::build(chat, area.width, anim_tick, now_ms));
    }
    let cache = viewport.as_ref().expect("viewport built above");
    let content_h = area.height as usize;
    let max_rows = cache.total_rows().saturating_sub(content_h);
    if follow {
        *scroll = max_rows as u32;
    }
    *scroll = (*scroll as usize).min(max_rows) as u32;
    let (start, end, top_skip) = cache.visible_window(*scroll as usize, content_h);
    let window = &cache.lines()[start..end];
    // A dropped leading row makes the wrapped-row `top_skip` meaningless
    // (it would skip rows of the wrong line) — drop the skip in that case.
    let top_skip = match window.first().map(|l| clean_text(&plain_text(l))) {
        Some(Some(_)) => top_skip,
        _ => 0,
    };
    let lines: Vec<Line<'static>> = window
        .iter()
        .filter_map(|l| clean_text(&plain_text(l)).map(Line::raw))
        .collect();
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para.scroll((top_skip as u16, 0)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::Config;

    fn keybindings() -> KeyBindings {
        KeyBindings::from_config(&Config::default())
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn is_active_truth_table() {
        assert!(!is_active(false, false));
        assert!(is_active(true, false));
        assert!(is_active(false, true));
        assert!(is_active(true, true));
    }

    #[test]
    fn toggle_key_flips_state() {
        let kb = keybindings();
        // Default copy-mode key is Ctrl+G.
        assert_eq!(next_state(&ctrl('g'), false, &kb), (true, true));
        assert_eq!(next_state(&ctrl('g'), true, &kb), (false, true));
    }

    #[test]
    fn active_mode_swallows_other_keys() {
        let kb = keybindings();
        // A plain letter is swallowed while copy mode is active.
        assert_eq!(next_state(&plain('x'), true, &kb), (true, true));
        assert_eq!(next_state(&plain('a'), true, &kb), (true, true));
    }

    #[test]
    fn esc_exits_active_mode() {
        let kb = keybindings();
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(next_state(&esc, true, &kb), (false, true));
    }

    #[test]
    fn inactive_passes_through_non_toggle_keys() {
        let kb = keybindings();
        assert_eq!(next_state(&plain('x'), false, &kb), (false, false));
        assert_eq!(
            next_state(&ctrl('g'), false, &kb),
            (true, true),
            "toggle must still fire when inactive"
        );
    }

    #[test]
    fn clean_text_drops_headers_separators_and_code_frames() {
        assert_eq!(clean_text("\u{276f} User:"), None);
        assert_eq!(clean_text("\u{276f} Say:"), None);
        assert_eq!(clean_text("\u{2500}\u{2500}\u{2500}\u{2500}"), None);
        assert_eq!(clean_text("\u{2501}\u{2501}\u{2501}"), None);
        assert_eq!(clean_text("\u{250c} rust "), None);
        assert_eq!(
            clean_text("\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}"),
            None
        );
    }

    #[test]
    fn clean_text_strips_gutter_and_prefixes_keeps_deeper_indent() {
        // 4-space (user/assistant) and 2-space (plan) gutters go…
        assert_eq!(clean_text("    hello"), Some("hello".into()));
        assert_eq!(clean_text("  plan line"), Some("plan line".into()));
        // …but indentation beyond the fixed slot survives.
        assert_eq!(clean_text("      nested"), Some("  nested".into()));
        // Code-frame `│ ` prefix and bare `│` empty row, blockquote `▎ `.
        assert_eq!(clean_text("\u{2502} fn main() {}"), Some("fn main() {}".into()));
        assert_eq!(clean_text("\u{2502}"), Some(String::new()));
        assert_eq!(clean_text("\u{258e} quoted"), Some("quoted".into()));
        // Gutter + code prefix compose (code inside an indented block).
        assert_eq!(
            clean_text("    \u{2502} code"),
            Some("code".into())
        );
        // Trailing padding from the border filler is trimmed.
        assert_eq!(clean_text("hi   "), Some("hi".into()));
    }

    #[test]
    fn clean_text_keeps_semantic_headers_and_plain_rows() {
        assert_eq!(
            clean_text("\u{25b8} bash ls -la"),
            Some("\u{25b8} bash ls -la".into())
        );
        assert_eq!(clean_text("\u{1f4ad} Thinking"), Some("\u{1f4ad} Thinking".into()));
        assert_eq!(clean_text("[image: a.png]"), Some("[image: a.png]".into()));
        assert_eq!(clean_text(""), Some(String::new()));
        // Two dashes are not a separator (e.g. an em-dash-less "--" flag text).
        assert_eq!(clean_text("--verbose"), Some("--verbose".into()));
    }

    #[test]
    fn render_clean_shows_code_text_without_frame() {
        use opencoder_session::SessionEvent;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta("```rust\nfn main() {}\n```".into()));
        v.apply(&SessionEvent::Done);

        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut scroll = 0u32;
        let mut viewport = None;
        terminal
            .draw(|f| {
                render_clean(f, f.area(), &v, &mut scroll, true, 0, 0, &mut viewport);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let text: String = buf
            .content
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(text.contains("fn main() {}"), "code text must survive: {text}");
        for deco in ['\u{250c}', '\u{2514}', '\u{2502}'] {
            assert!(!text.contains(deco), "code frame {deco:?} must be stripped: {text}");
        }
        assert!(
            !text.contains("\u{276f} Say:"),
            "role header must be dropped: {text}"
        );
    }
}
