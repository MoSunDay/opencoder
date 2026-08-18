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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
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
/// The toggle is global: it fires even while a plan-edit/notepad overlay is
/// open — the overlay's clean full-screen view comes from
/// `render_composer_clean` / `render_notepad_clean`, so Ctrl+G is never a
/// dead key. While active every key is swallowed; the toggle key and `Esc`
/// exit, and the overlay is left intact underneath (layered modality: the
/// first Esc leaves copy mode, the next one reaches the overlay).
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
///
/// When `reserve_chip_row` is set (overlay editors: annotation/plan), the
/// first row is left blank because render.rs pins the COPY MODE chip to
/// `area.y`; text starts on the second row so a long first line's tail is
/// never overpainted and stays selectable. The plain composer passes false
/// — its input rows are short and the transcript is the copy target.
pub(crate) fn render_composer_clean(
    f: &mut Frame,
    area: Rect,
    input: &str,
    reserve_chip_row: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let text_area = if reserve_chip_row && area.height > 1 {
        Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        }
    } else {
        area
    };
    let rows = crate::composer::wrap_rows(input, area.width, 0);
    let chars: Vec<char> = input.chars().collect();
    let lines: Vec<Line<'static>> = rows
        .iter()
        .map(|vr| Line::raw(chars[vr.start..vr.end].iter().collect::<String>()))
        .collect();
    f.render_widget(Paragraph::new(lines), text_area);
}

/// Draw the "COPY MODE" status chip into `area`: the notepad fullscreen
/// branch in render.rs returns before the shared status-chip pass runs, so
/// without this copy mode over the notepad would be invisible. Minimal
/// replica of render.rs's private `render_status_chip` styling, pinned to
/// the last row (right-aligned) so file text at column 0 stays selectable.
fn render_copy_chip(f: &mut Frame, area: Rect) {
    let text = "COPY MODE: Ctrl+G/Esc";
    let chip_w = (crate::composer::str_width(text) as u16).saturating_add(2);
    let w = chip_w.min(area.width);
    let chip_rect = Rect {
        x: area.x + area.width.saturating_sub(w).saturating_sub(1),
        y: area.bottom().saturating_sub(1),
        width: w,
        height: 1,
    };
    f.render_widget(Clear, chip_rect);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {text} "),
            Style::default()
                .fg(Color::Black)
                .bg(crate::theme::warn_color())
                .add_modifier(Modifier::BOLD),
        ))),
        chip_rect,
    );
}

/// Render the notepad for copy mode: the editor buffer's visual rows fill
/// the whole `area` — no tree panel, no block/border, no line-number
/// gutter, no cmdline row — so terminal-native selection copies pure file
/// text. Wrapping comes from [`crate::notepad::editor::row_texts`], the
/// same layout model the decorated editor renders from. A COPY MODE chip
/// is pinned to the last row so the mode stays visible (render.rs's shared
/// chip pass never runs for the notepad fullscreen branch).
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
    render_copy_chip(f, area);
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

    /// The toggle is global: `handle_key` has no overlay knowledge anymore,
    /// so Ctrl+G flips copy mode on and back off even while the caller has
    /// a plan-edit/notepad overlay open (the overlay survives underneath).
    #[test]
    fn toggle_fires_even_with_overlay_open() {
        let kb = keybindings();
        let mut active = false;
        assert!(handle_key(&ctrl('g'), &mut active, &kb));
        assert!(active, "toggle must fire even with an overlay open");
        assert!(handle_key(&ctrl('g'), &mut active, &kb));
        assert!(!active, "toggle must exit an active copy mode");
    }

    /// While copy mode is active every key is swallowed (consumed, state
    /// kept); Esc is consumed and exits — the overlay underneath stays open
    /// and receives the *next* Esc (layered modality).
    #[test]
    fn active_swallows_keys_and_esc_exits() {
        let kb = keybindings();
        let mut active = true;
        assert!(handle_key(&plain('x'), &mut active, &kb));
        assert!(active, "plain keys are swallowed while copy mode is active");
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(handle_key(&esc, &mut active, &kb));
        assert!(!active, "Esc must be consumed and exit copy mode");
        assert!(
            !handle_key(&plain('x'), &mut active, &kb),
            "exited copy mode passes keys through again"
        );
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
            .draw(|f| render_composer_clean(f, f.area(), "hello\nworld", false))
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

    /// Overlay editors (annotation/plan) reserve row 0 for the COPY MODE
    /// chip render.rs pins to the composer's first row: text must start on
    /// row 1 so a long first line's tail is never overpainted.
    #[test]
    fn render_composer_clean_reserved_row0_stays_blank_for_chip() {
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|f| render_composer_clean(f, f.area(), "hello\nworld", true))
            .unwrap();
        let rows = buf_rows(terminal.backend().buffer());
        assert!(
            rows[0].trim_end().is_empty(),
            "row0 reserved for the chip, must be blank: {:?}",
            rows[0]
        );
        assert!(rows[1].starts_with("hello"), "row1: {:?}", rows[1]);
        assert!(rows[2].starts_with("world"), "row2: {:?}", rows[2]);
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

    /// The notepad fullscreen branch in render.rs returns before the shared
    /// status-chip pass, so the clean view must paint its own COPY MODE
    /// cue — otherwise copy mode over the notepad is invisible.
    #[test]
    fn render_notepad_clean_shows_copy_mode_chip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha-line\nbeta-line\n").unwrap();
        let mut view = crate::notepad::NotepadView::new(dir.path().to_path_buf());
        view.editor.load(&dir.path().join("a.txt"));

        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|f| render_notepad_clean(f, f.area(), &view))
            .unwrap();
        let rows = buf_rows(terminal.backend().buffer());
        assert!(
            rows[7].contains("COPY MODE: Ctrl+G/Esc"),
            "last row must carry the COPY MODE chip: {:?}",
            rows[7]
        );
        assert!(
            rows[0].starts_with("alpha-line"),
            "file text must stay flush at column 0: {:?}",
            rows[0]
        );
        let all = rows.concat();
        for deco in ['\u{250c}', '\u{2514}', '\u{2502}'] {
            assert!(
                !all.contains(deco),
                "chip must not reintroduce chrome {deco:?}: {all:?}"
            );
        }
    }
}
