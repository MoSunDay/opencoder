//! Cursor ownership regression tests for popup text fields.
//!
//! ratatui's `Frame::set_cursor_position` is last-write-wins within a frame:
//! the composer cursor call at the end of `render` used to override the
//! cursor that the `/cli` and `/mcp` popups had placed on their own editable
//! fields, so those fields showed no cursor at all (the caret silently sat
//! in the composer below). `render` therefore skips the composer cursor
//! while any text-owning popup is open — these tests pin that guard.

use super::*;
use crate::cli_menu::{CliField, CliForm, CliMenu, ContentDialog};
use crate::mcp_menu::{McpForm, McpMenu};

/// Render one full frame with the given popups and benign defaults.
fn draw(
    terminal: &mut Terminal<TestBackend>,
    cli_menu: Option<&CliMenu>,
    mcp_menu: Option<&McpMenu>,
) {
    let chat = ChatView::default();
    let mut scroll = 0u32;
    let mut queue_scroll = 0u32;
    let mut hits = MouseHits::default();
    let mut viewport: Option<ViewportCache> = None;
    render(
        terminal,
        &chat,
        "hi",
        2,
        &Line::raw("title"),
        false,
        0,
        0,
        200_000,
        200_000,
        "idle",
        &[],
        &[],
        &mut scroll,
        true,
        &mut queue_scroll,
        0,
        0,
        None,          // mode_flash
        None,          // skill_menu
        None,          // task_picker
        None,          // command_menu
        None,          // model_menu
        mcp_menu,
        None,          // envs_menu
        cli_menu,
        None,          // skill_toggle_menu
        None,          // ap_menu
        None,          // cache_salt_menu
        None,          // keymap_menu
        None,          // question_menu
        &mut hits,
        &mut viewport,
        false,
        false,
        &[],
        false,
        None,
        None,
        0,
        0,
        true,
        opencoder_core::ApMode::Off,
        "act",
        None,
    )
    .unwrap();
}

/// Read the terminal cursor position the last frame left behind.
fn cursor(terminal: &mut Terminal<TestBackend>) -> (u16, u16) {
    let p = terminal.backend_mut().get_cursor_position().unwrap();
    (p.x, p.y)
}

/// While the `/cli` multi-line content dialog is open, the caret must sit at
/// the dialog's logical cursor position. The dialog rect depends only on the
/// frame size, so on 80x24: w = min(72, 78) = 72 -> x = 4;
/// h = min(VIEW_LINES + 2, 23) = 10 -> y = 7. Text "hello\nworld" with the
/// cursor at char 8 is logical (line 1, col 2), so the caret must be at
/// (4 + 1 + 2, 7 + 1 + 1) = (7, 9). Before the guard fix the composer call
/// overwrote this with its own position.
#[test]
fn cli_content_dialog_keeps_cursor_inside_dialog() {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut form = CliForm::new_blank();
    form.field = CliField::Content;
    form.content = "hello\nworld".into();
    form.content_cursor = 8;
    form.content_dialog = Some(ContentDialog::new("hello\nworld".into(), 8));
    let menu = CliMenu::Form(form);
    draw(&mut terminal, Some(&menu), None);
    terminal.backend_mut().assert_cursor_position((7, 9));
}

/// The bug shape: with the popup open the caret used to land exactly on the
/// composer's edit position (same coordinates as with no popup open). The
/// guard must move the caret off that spot — and, since the form popup is
/// anchored above the composer, strictly above it.
#[test]
fn cli_form_field_moves_cursor_off_composer() {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    draw(&mut terminal, None, None);
    let composer = cursor(&mut terminal);

    let mut form = CliForm::new_blank();
    form.name = "abc".into();
    form.name_cursor = 3;
    let menu = CliMenu::Form(form);
    draw(&mut terminal, Some(&menu), None);
    let pos = cursor(&mut terminal);

    assert_ne!(
        pos, composer,
        "caret must leave the composer while the /cli form is open"
    );
    assert!(
        pos.1 < composer.1,
        "caret must sit above the composer inside the /cli form (got {pos:?}, composer {composer:?})"
    );
}

/// Same guard covers the `/mcp` form: its fields own the caret while open,
/// so the composer position must not win the frame.
#[test]
fn mcp_form_field_moves_cursor_off_composer() {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    draw(&mut terminal, None, None);
    let composer = cursor(&mut terminal);

    let mut form = McpForm::new_blank();
    form.name = "abc".into();
    form.name_cursor = 3;
    let menu = McpMenu::Form(form);
    draw(&mut terminal, None, Some(&menu));
    let pos = cursor(&mut terminal);

    assert_ne!(
        pos, composer,
        "caret must leave the composer while the /mcp form is open"
    );
    assert!(
        pos.1 < composer.1,
        "caret must sit above the composer inside the /mcp form (got {pos:?}, composer {composer:?})"
    );
}
