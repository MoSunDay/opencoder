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
//! stripped — no rounded border, no scrollbar column, no `[turn cost]` row,
//! no border indicators; per row the indent gutter and code-frame prefixes
//! (`│ `, `▎ `) are removed and pure-decoration rows (role headers
//! `❯ User:`/`❯ Say:`, thematic breaks, `┌ lang`/`└───` code frames, plan
//! headers) are dropped. Stripping is *structured*: [`clean`] matches the
//! exact span shapes the renderers declare as constants, and the scroll
//! geometry runs on the cleaned line set ([`CleanModel`]), so dropped rows
//! leave no blank band and the first visible row is never over-skipped.
//! The app itself never touches the clipboard — copying is the terminal's
//! own job via its native selection shortcuts.

pub mod clean;

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
///
/// While an overlay (plan editor / notepad) is open (`overlay_active`), copy
/// mode yields entirely: no toggle, no swallowing, so the overlay receives
/// every key — otherwise Ctrl+G would be a silent dead zone (the copy chip
/// is not rendered in those views).
pub(crate) fn handle_key(
    k: &KeyEvent,
    copy_mode: &mut bool,
    keymap: &KeyBindings,
    overlay_active: bool,
) -> bool {
    if overlay_active {
        return false;
    }
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

/// Render the transcript for copy mode: full width, no block/border, no
/// scrollbar, no `[turn cost]` row, no border indicators — every visible row
/// is already clean text so terminal-native selection spans it directly.
/// Reuses the shared viewport cache; the width check naturally rebuilds it
/// when toggling in/out of copy mode (full width vs inner).
///
/// Scrolling geometry runs on the *cleaned* line set (`ViewportCache::
/// cleaned`), so follow/clamp counts and the visible window both measure
/// post-decoration rows: a window full of dropped decoration rows renders
/// the following content rows instead of leaving a blank band, and `top_skip`
/// is always the in-line offset of the first *clean* row (the old
/// "first row was dropped, discard top_skip" patch is gone because the
/// window math can no longer land on a dropped row).
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
    let cache = viewport.as_mut().expect("viewport built above");
    let cleaned = cache.cleaned(area.width);
    let content_h = area.height as usize;
    let max_rows = cleaned.total_rows().saturating_sub(content_h);
    if follow {
        *scroll = max_rows as u32;
    }
    *scroll = (*scroll as usize).min(max_rows) as u32;
    let (start, end, top_skip) = cleaned.visible_window(*scroll as usize, content_h);
    let lines: Vec<Line<'static>> = cleaned.texts()[start..end]
        .iter()
        .map(|t| Line::raw(t.clone()))
        .collect();
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para.scroll((top_skip as u16, 0)), area);
}

/// Render the composer input for copy mode: fill `area` with the raw
/// wrapped text rows — no block/border, no prompt glyph, no continuation
/// padding, no attachment badge — so terminal-native selection spans
/// exactly the typed text. Mirrors [`render_clean`] for the input pane;
/// the shared `composer::wrap_rows` model keeps wrapping identical to the
/// decorated composer.
pub(crate) fn render_composer_clean(f: &mut Frame, area: Rect, input: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = crate::composer::wrap_rows(input, area.width, 0);
    let chars: Vec<char> = input.chars().collect();
    let lines: Vec<Line<'static>> = rows
        .iter()
        .map(|vr| Line::raw(chars[vr.start..vr.end].iter().collect::<String>()))
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

/// Render the notepad for copy mode: the editor buffer's visual rows fill
/// the whole `area` — no tree panel, no block/border, no line-number
/// gutter, no cmdline row — so terminal-native selection copies pure file
/// text. Wrapping comes from [`crate::notepad::editor::row_texts`], the
/// same layout model the decorated editor renders from.
pub(crate) fn render_notepad_clean(f: &mut Frame, area: Rect, view: &crate::notepad::NotepadView) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = crate::notepad::editor::row_texts(&view.editor.vim.text, area.width);
    // `row_texts` carries each logical line's terminating newline for exact
    // round-trips; a Line must not embed it — the terminal already breaks
    // rows — so trim it back off here.
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .map(|r| Line::raw(r.trim_end_matches('\n').to_owned()))
        .collect();
    f.render_widget(Paragraph::new(lines), area);
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

    /// While an overlay (plan edit / notepad) is open, the copy-mode toggle
    /// key must not fire: the state stays unchanged and the key passes through
    /// so the overlay can handle it.
    #[test]
    fn overlay_active_ignores_toggle_key() {
        let kb = keybindings();
        let mut active = false;
        assert!(!handle_key(&ctrl('g'), &mut active, &kb, true));
        assert!(!active, "overlay must block the copy-mode toggle");

        let mut active = true;
        assert!(!handle_key(&ctrl('g'), &mut active, &kb, true));
        assert!(
            active,
            "overlay must not toggle an already-active copy mode"
        );
    }

    #[test]
    fn overlay_inactive_toggles_normally() {
        let kb = keybindings();
        let mut active = false;
        assert!(handle_key(&ctrl('g'), &mut active, &kb, false));
        assert!(active, "toggle must fire when no overlay is open");
    }

    /// With an overlay open, even an already-active copy mode must not swallow
    /// keys — the overlay receives them (otherwise Ctrl+G is a dead zone).
    #[test]
    fn overlay_active_does_not_swallow_when_copy_mode_active() {
        let kb = keybindings();
        let mut active = true;
        assert!(!handle_key(&plain('x'), &mut active, &kb, true));
        assert!(
            active,
            "copy mode must stay armed for after the overlay closes"
        );
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!handle_key(&esc, &mut active, &kb, true));
        assert!(active, "Esc must reach the overlay, not exit copy mode");
    }

    // ── Render-level fixtures ─────────────────────────────────────────────

    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    /// Draw `view` through [`render_clean`] on a 40×8 terminal (follow mode)
    /// and return the terminal for buffer inspection.
    fn draw_clean(view: &ChatView) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        let mut scroll = 0u32;
        let mut viewport = None;
        terminal
            .draw(|f| {
                render_clean(f, f.area(), view, &mut scroll, true, 0, 0, &mut viewport);
            })
            .unwrap();
        terminal
    }

    /// Per-row cell contents of a drawn buffer.
    fn buf_rows(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// All cell symbols joined into one string.
    fn buf_text(buf: &Buffer) -> String {
        buf.content
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect()
    }

    fn no_code_frame_glyphs(text: &str) {
        for deco in ['\u{250c}', '\u{2514}', '\u{2502}'] {
            assert!(
                !text.contains(deco),
                "code frame {deco:?} must be stripped: {text}"
            );
        }
    }

    #[test]
    fn render_clean_shows_code_text_without_frame() {
        use opencoder_session::SessionEvent;

        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta(
            "```rust\nfn main() {}\n```".into(),
        ));
        v.apply(&SessionEvent::Done);

        let terminal = draw_clean(&v);
        let text = buf_text(terminal.backend().buffer());
        assert!(
            text.contains("fn main() {}"),
            "code text must survive: {text}"
        );
        no_code_frame_glyphs(&text);
        assert!(
            !text.contains("\u{276f} Say:"),
            "role header must be dropped: {text}"
        );
    }

    #[test]
    fn render_clean_keeps_separator_like_code_rows() {
        use opencoder_session::SessionEvent;

        // YAML frontmatter inside a fenced block: every `---` row carries a
        // `│ ` code prefix span, so it is content — the old text heuristic
        // killed it as a "separator".
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta(
            "```yaml\n---\ntitle: x\n---\n```".into(),
        ));
        v.apply(&SessionEvent::Done);

        let terminal = draw_clean(&v);
        let text = buf_text(terminal.backend().buffer());
        assert!(
            text.contains("---"),
            "frontmatter fences must survive: {text}"
        );
        assert!(
            text.contains("title: x"),
            "frontmatter body must survive: {text}"
        );
        no_code_frame_glyphs(&text);
    }

    #[test]
    fn render_clean_keeps_text_leading_spaces_beyond_gutter() {
        use ratatui::text::Span;

        // A plan-body row (2-space gutter span) whose text itself starts
        // with two spaces: only the gutter span goes — the old heuristic
        // stripped 4 first and ate the text's own indentation.
        let mut v = ChatView::default();
        v.push_marker(Line::from(vec![Span::raw("  "), Span::raw("  nested")]));

        let terminal = draw_clean(&v);
        let rows = buf_rows(terminal.backend().buffer());
        assert_eq!(rows[0].trim_end(), "  nested", "own lead kept, gutter gone");
    }

    #[test]
    fn render_clean_no_blank_band_under_trailing_decoration() {
        use opencoder_session::SessionEvent;

        // Follow-mode clamp must use the *cleaned* row count: with the tail
        // of the transcript full of dropped decoration rows (frames, role
        // header, trailing blank), the old decorated-geometry scroll left a
        // blank band instead of pinning the last content row.
        let body: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        let mut v = ChatView::default();
        v.apply(&SessionEvent::TextDelta(format!("```rust\n{body}```")));
        v.apply(&SessionEvent::Done);

        let terminal = draw_clean(&v);
        let rows = buf_rows(terminal.backend().buffer());
        assert!(
            rows[0].starts_with("line4"),
            "window must start at clean row 3 (line4), got {:?}",
            rows[0]
        );
        assert!(
            rows[6].starts_with("line10"),
            "last content row must pin at row 6, got {:?}",
            rows[6]
        );
        assert!(
            rows[7].trim_end().is_empty(),
            "only the single structural blank may trail, got {:?}",
            rows[7]
        );
    }

    #[test]
    fn render_composer_clean_shows_text_without_chrome() {
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|f| render_composer_clean(f, f.area(), "hello\nworld"))
            .unwrap();
        let rows = buf_rows(terminal.backend().buffer());
        // Text rows land flush at column 0: no border, no prompt glyph.
        assert!(
            rows[0].starts_with("hello"),
            "row0 flush left: {:?}",
            rows[0]
        );
        assert!(
            rows[1].starts_with("world"),
            "row1 flush left: {:?}",
            rows[1]
        );
        let all = rows.concat();
        assert!(!all.contains('\u{276f}'), "no prompt glyph: {all:?}");
        for deco in ['\u{250c}', '\u{2514}', '\u{2500}'] {
            assert!(
                !all.contains(deco),
                "border {deco:?} must be absent: {all:?}"
            );
        }
    }

    #[test]
    fn render_notepad_clean_shows_file_text_without_chrome() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha-line\nbeta-line\n").unwrap();
        let mut view = crate::notepad::NotepadView::new(dir.path().to_path_buf());
        view.editor.load(&dir.path().join("a.txt"));

        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|f| render_notepad_clean(f, f.area(), &view))
            .unwrap();
        let rows = buf_rows(terminal.backend().buffer());
        // File text flush at column 0 — the decorated editor would put a
        // line-number gutter there instead.
        assert!(rows[0].starts_with("alpha-line"), "row0: {:?}", rows[0]);
        assert!(rows[1].starts_with("beta-line"), "row1: {:?}", rows[1]);
        let all = rows.concat();
        for deco in ['\u{250c}', '\u{2514}', '\u{2502}'] {
            assert!(
                !all.contains(deco),
                "decoration {deco:?} must be absent: {all:?}"
            );
        }
    }
}
