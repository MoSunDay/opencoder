//! Notepad integration helpers for the main event loop.
//!
//! Extracted from `app.rs` to keep that file under the 800-line iteration cap.
//! Bridges the notepad's tree/editor with the host app loop via key dispatch
//! and focus toggling between the notepad top region and the chat composer.

use crossterm::event::KeyEvent;

use crate::keymap::KeyBindings;
use crate::notepad::NotepadView;

/// Result of processing a key when the notepad is open.
pub(crate) struct KeyResult {
    /// Whether the key was consumed (notepad handled it).
    pub handled: bool,
}

/// Dispatch a key when the notepad is active.
///
/// - When `np_chat_focus` is `true`, only the focus-toggle key (Ctrl+O) is
///   intercepted (it returns focus to the notepad); all other keys fall
///   through to the composer.
/// - When `np_chat_focus` is `false`, keys go to the notepad tree/editor.
///   The focus-toggle key switches focus to the chat composer.
/// - Esc in notepad focus closes the notepad (handled by `dispatch_key`).
pub(crate) async fn key(
    notepad: &mut Option<NotepadView>,
    k: KeyEvent,
    keymap: &KeyBindings,
    np_chat_focus: &mut bool,
) -> KeyResult {
    if notepad.is_none() {
        return KeyResult { handled: false };
    }

    // toggle_focus works in both directions.
    if keymap.toggle_focus.matches(&k) {
        *np_chat_focus = !*np_chat_focus;
        return KeyResult { handled: true };
    }

    // When chat is focused, let keys fall through to the composer.
    if *np_chat_focus {
        return KeyResult { handled: false };
    }

    // Dispatch to notepad (Esc in notepad focus will close it).
    crate::notepad::dispatch_key(notepad, k).await;
    if notepad.is_none() {
        // Notepad was closed by Esc — reset chat focus.
        *np_chat_focus = false;
    }
    KeyResult { handled: true }
}

/// Handle `KeyAction::Bash(cmd)`: echo the command, spawn execution, push
/// a placeholder Tool block into the chat.
pub(crate) fn handle_bash(
    cmd: &str,
    chat: &mut crate::chat::ChatView,
    bash_rx: &mut Option<tokio::sync::oneshot::Receiver<String>>,
    workdir: &std::path::Path,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
) {
    crate::app_helpers::push_user(chat, history, hist_idx, &format!("!{cmd}"));
    *bash_rx = Some(crate::bash_exec::spawn(cmd, workdir));
    chat.push_bash_tool(cmd);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepad::{Focus, NotepadView};
    use crossterm::event::{KeyCode, KeyModifiers};

    fn make_view(dir: &std::path::Path) -> NotepadView {
        NotepadView::new(dir.to_path_buf())
    }

    #[tokio::test]
    async fn key_unhandled_when_notepad_closed() {
        let mut np: Option<NotepadView> = None;
        let mut focus = false;
        let km = KeyBindings::default();
        let r = key(&mut np, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &km, &mut focus).await;
        assert!(!r.handled);
    }

    #[tokio::test]
    async fn toggle_focus_switches_to_chat() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut focus = false;
        let km = KeyBindings::default();
        let k = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
        let r = key(&mut np, k, &km, &mut focus).await;
        assert!(r.handled);
        assert!(focus, "np_chat_focus should be true after toggle");
    }

    #[tokio::test]
    async fn toggle_focus_switches_back_to_notepad() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut focus = true;
        let km = KeyBindings::default();
        let k = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
        let r = key(&mut np, k, &km, &mut focus).await;
        assert!(r.handled);
        assert!(!focus, "np_chat_focus should be false after toggle back");
    }

    #[tokio::test]
    async fn keys_fall_through_when_chat_focused() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut focus = true;
        let km = KeyBindings::default();
        let r =
            key(&mut np, KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &km, &mut focus)
                .await;
        assert!(!r.handled, "non-toggle key should fall through to composer");
    }

    #[tokio::test]
    async fn esc_closes_notepad() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut focus = false;
        let km = KeyBindings::default();
        let r =
            key(&mut np, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &km, &mut focus).await;
        assert!(r.handled);
        assert!(np.is_none(), "notepad should be closed");
        assert!(!focus, "chat focus should be reset");
    }

    #[tokio::test]
    async fn tab_cycles_focus_in_notepad() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut focus = false;
        let km = KeyBindings::default();
        let r =
            key(&mut np, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &km, &mut focus).await;
        assert!(r.handled);
        assert_eq!(np.as_ref().unwrap().focus, Focus::Editor);
    }
}

/// Handle mouse events for the notepad divider drag.
///
/// Returns `true` when the event was consumed (caller should set dirty + continue).
pub(crate) fn handle_notepad_drag(
    m: &crossterm::event::MouseEvent,
    hits: &crate::render::MouseHits,
    notepad: &mut Option<crate::notepad::NotepadView>,
    np_drag: &mut Option<(u16, u16)>,
) -> bool {
    use crossterm::event::{MouseButton, MouseEventKind};
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(np) = notepad.as_mut() {
                if let Some(div) = hits.divider {
                    if m.row >= div.y && m.row < div.y + div.height {
                        *np_drag = Some((m.row, np.height));
                        return true;
                    }
                }
            }
            false
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let (Some(np), Some((start_row, start_height))) = (notepad.as_mut(), np_drag) {
                let delta = *start_row as i32 - m.row as i32;
                np.height = (*start_height as i32 + delta).max(5) as u16;
                return true;
            }
            false
        }
        MouseEventKind::Up(MouseButton::Left) => np_drag.take().is_some(),
        _ => false,
    }
}

/// Poll a background bash command (`!cmd`) and fill the chat Tool block.
///
/// Returns `true` when the chat state changed (caller should set dirty).
pub(crate) fn poll_bash(
    bash_rx: &mut Option<tokio::sync::oneshot::Receiver<String>>,
    chat: &mut crate::chat::ChatView,
) -> bool {
    use tokio::sync::oneshot::error::TryRecvError;
    let rx = match bash_rx.as_mut() {
        Some(r) => r,
        None => return false,
    };
    match rx.try_recv() {
        Ok(out) => {
            chat.finish_bash_tool(&out);
            *bash_rx = None;
            true
        }
        Err(TryRecvError::Empty) => false,
        Err(TryRecvError::Closed) => {
            chat.finish_bash_tool("(command aborted)");
            *bash_rx = None;
            true
        }
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    use crate::notepad::NotepadView;
    use crate::render::MouseHits;
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    fn make_view(dir: &std::path::Path) -> NotepadView {
        NotepadView::new(dir.to_path_buf())
    }

    fn empty_hits() -> MouseHits {
        MouseHits {
            divider: None,
            ..Default::default()
        }
    }

    #[test]
    fn drag_starts_on_divider_click() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut drag = None;
        let mut hits = empty_hits();
        hits.divider = Some(Rect::new(0, 10, 80, 1));
        let m = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(handle_notepad_drag(&m, &hits, &mut np, &mut drag));
        assert!(drag.is_some());
        assert_eq!(drag.unwrap().0, 10); // start_row
    }

    #[test]
    fn drag_adjusts_height() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let start_h = np.as_ref().unwrap().height;
        let mut drag = Some((10u16, start_h));
        let hits = empty_hits();
        // Drag UP by 3 rows → height increases by 3.
        let m = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 5,
            row: 7,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(handle_notepad_drag(&m, &hits, &mut np, &mut drag));
        assert_eq!(np.as_ref().unwrap().height, start_h + 3);
    }

    #[test]
    fn drag_down_decreases_height() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let start_h = np.as_ref().unwrap().height;
        let mut drag = Some((10u16, start_h));
        let hits = empty_hits();
        let m = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 5,
            row: 14,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(handle_notepad_drag(&m, &hits, &mut np, &mut drag));
        assert_eq!(np.as_ref().unwrap().height, start_h - 4);
    }

    #[test]
    fn mouse_up_ends_drag() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut drag = Some((10u16, 15u16));
        let hits = empty_hits();
        let m = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 5,
            row: 12,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(handle_notepad_drag(&m, &hits, &mut np, &mut drag));
        assert!(drag.is_none());
    }

    #[test]
    fn drag_clamps_to_minimum_5() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        // Start with height 5, drag down a lot.
        let mut drag = Some((10u16, 5u16));
        let hits = empty_hits();
        let m = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 5,
            row: 50,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        handle_notepad_drag(&m, &hits, &mut np, &mut drag);
        assert!(np.as_ref().unwrap().height >= 5);
    }

    #[test]
    fn click_outside_divider_not_consumed() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let mut drag = None;
        let mut hits = empty_hits();
        hits.divider = Some(Rect::new(0, 10, 80, 1));
        // Click at row 5, which is above the divider at row 10.
        let m = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(!handle_notepad_drag(&m, &hits, &mut np, &mut drag));
        assert!(drag.is_none());
    }
}
