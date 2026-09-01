//! Notepad integration helpers for the main event loop.
//!
//! Extracted from `app.rs` to keep that file under the 800-line iteration cap.
//! Bridges the notepad's tree/editor with the host app loop via key dispatch.

use crossterm::event::KeyEvent;

use crate::notepad::NotepadView;

/// Result of processing a key when the notepad is open.
pub(crate) struct KeyResult {
    /// Whether the key was consumed (notepad handled it).
    pub handled: bool,
}

/// Dispatch a key when the notepad is active.
///
/// The notepad is a fullscreen file viewer/editor: every key goes to the
/// tree/editor, and `Esc` (handled by `dispatch_key`) closes it and returns
/// to the normal chat view.
pub(crate) async fn key(notepad: &mut Option<NotepadView>, k: KeyEvent) -> KeyResult {
    if notepad.is_none() {
        return KeyResult { handled: false };
    }
    crate::notepad::dispatch_key(notepad, k).await;
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
    let echoed = format!("!{cmd}");
    crate::app_helpers::push_user(chat, history, hist_idx, &echoed, &echoed);
    *bash_rx = Some(crate::bash_exec::spawn(cmd, workdir));
    chat.push_bash_tool(cmd);
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
        let r = key(&mut np, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
        assert!(!r.handled);
    }

    #[tokio::test]
    async fn key_consumed_when_notepad_open() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let r = key(
            &mut np,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        )
        .await;
        assert!(r.handled, "key should be handled by the notepad");
    }

    #[tokio::test]
    async fn ctrl_o_has_no_special_meaning() {
        // The toggle_focus shortcut was removed: notepad is fullscreen and
        // Ctrl+O is just a normal editor keypress (consumed, notepad stays).
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let r = key(
            &mut np,
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
        )
        .await;
        assert!(r.handled);
        assert!(np.is_some(), "notepad must remain open (no chat toggle)");
    }

    #[tokio::test]
    async fn esc_closes_notepad() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let r = key(&mut np, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert!(r.handled);
        assert!(np.is_none(), "notepad should be closed");
    }

    #[tokio::test]
    async fn tab_cycles_focus_in_notepad() {
        let d = tempfile::tempdir().unwrap();
        let mut np: Option<NotepadView> = Some(make_view(d.path()));
        let r = key(&mut np, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await;
        assert!(r.handled);
        assert_eq!(np.as_ref().unwrap().focus, Focus::Editor);
    }
}
