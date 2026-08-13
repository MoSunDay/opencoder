//! Mouse-click handling for the keymap modal (Ctrl+H) bottom button bar.
//!
//! Buttons are registered as screen-space `Rect`s during rendering
//! (see [`view::render_keymap_popup`]). When a left-click lands inside one,
//! we reuse the same button-activation logic as the keyboard path
//! (see [`state::activate_button`]).

use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::Rect;

use crate::keymap_menu::state::{activate_button, KeymapMenu, KeymapOutcome};

/// Handle a mouse event while the keymap modal is open.
///
/// Returns the same [`KeymapOutcome`] variants that the keyboard path
/// produces, so the caller's side-effect application is identical for both.
pub(crate) fn handle_keymap_mouse(
    menu: &mut Option<KeymapMenu>,
    btn_rects: &[Rect],
    col: u16,
    row: u16,
    kind: &MouseEventKind,
) -> KeymapOutcome {
    let Some(m) = menu.as_mut() else {
        return KeymapOutcome::Idle;
    };

    // Overlay open: ignore mouse, must use keyboard to confirm/close.
    if m.confirm_reset_open() || m.help_open() {
        return KeymapOutcome::Idle;
    }

    // Only react to left-click-down (ignore wheel, drag, etc.).
    if !matches!(kind, MouseEventKind::Down(MouseButton::Left)) {
        return KeymapOutcome::Idle;
    }

    // Hit-test against each registered button Rect.
    for (idx, r) in btn_rects.iter().enumerate() {
        if crate::render::in_rect(*r, col, row) {
            m.select_button_for_click(idx);
            return activate_button(menu, idx);
        }
    }

    KeymapOutcome::Idle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::MouseButton;
    use opencoder_core::KeymapConfig;

    fn make_menu() -> KeymapMenu {
        KeymapMenu::new(&KeymapConfig::default())
    }

    fn sample_rects() -> Vec<Rect> {
        vec![
            Rect::new(10, 5, 8, 1),
            Rect::new(21, 5, 12, 1),
            Rect::new(36, 5, 8, 1),
        ]
    }

    fn left_down(col: u16, row: u16) -> (u16, u16, MouseEventKind) {
        (col, row, MouseEventKind::Down(MouseButton::Left))
    }

    #[test]
    fn click_exit_button_quits() {
        let mut menu = Some(make_menu());
        let (col, row, kind) = left_down(12, 5);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), col, row, &kind);
        assert_eq!(out, KeymapOutcome::Quit);
        assert!(menu.is_none());
    }

    #[test]
    fn click_reset_button_opens_confirm() {
        let mut menu = Some(make_menu());
        let (col, row, kind) = left_down(25, 5);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), col, row, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().confirm_reset_open());
    }

    #[test]
    fn click_help_button_opens_overlay() {
        let mut menu = Some(make_menu());
        let (col, row, kind) = left_down(38, 5);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), col, row, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.as_ref().unwrap().help_open());
    }

    #[test]
    fn click_blank_area_is_idle() {
        let mut menu = Some(make_menu());
        let (col, row, kind) = left_down(0, 0);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), col, row, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.is_some());
    }

    #[test]
    fn mouse_ignored_when_confirm_reset_open() {
        let mut menu = Some(make_menu());
        menu.as_mut().unwrap().select_button_for_click(1);
        super::activate_button(&mut menu, 1);
        assert!(menu.as_ref().unwrap().confirm_reset_open());
        let (col, row, kind) = left_down(12, 5);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), col, row, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.is_some());
        assert!(menu.as_ref().unwrap().confirm_reset_open());
    }

    #[test]
    fn mouse_ignored_when_help_open() {
        let mut menu = Some(make_menu());
        super::activate_button(&mut menu, 2);
        assert!(menu.as_ref().unwrap().help_open());
        let (col, row, kind) = left_down(12, 5);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), col, row, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
        assert!(menu.is_some());
    }

    #[test]
    fn scroll_up_is_idle() {
        let mut menu = Some(make_menu());
        let kind = MouseEventKind::ScrollUp;
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), 12, 5, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
    }

    #[test]
    fn right_click_is_idle() {
        let mut menu = Some(make_menu());
        let kind = MouseEventKind::Down(MouseButton::Right);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), 12, 5, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
    }

    #[test]
    fn mouse_drag_is_idle() {
        let mut menu = Some(make_menu());
        let kind = MouseEventKind::Drag(MouseButton::Left);
        let out = handle_keymap_mouse(&mut menu, &sample_rects(), 12, 5, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
    }

    #[test]
    fn empty_rects_returns_idle() {
        let mut menu = Some(make_menu());
        let empty: Vec<Rect> = vec![];
        let (col, row, kind) = left_down(12, 5);
        let out = handle_keymap_mouse(&mut menu, &empty, col, row, &kind);
        assert_eq!(out, KeymapOutcome::Idle);
    }
}
