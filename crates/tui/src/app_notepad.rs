//! Notepad console integration helpers for the main event loop.
//!
//! Extracted from `app.rs` to keep that file under the 800-line iteration cap.
//! These free functions bridge the notepad's vim-console with the host app
//! loop: key dispatch, paste routing, background bash polling, and render.

use anyhow::Result;
use crossterm::event::KeyEvent;
use tokio::sync::oneshot;

use crate::keymap::KeyBindings;
use crate::notepad::{Focus, NotepadOutcome, NotepadView};
use crate::render::Term;

/// Result of processing a key in the notepad.
pub(crate) struct KeyResult {
    /// Whether the key was consumed (notepad was active and handled it).
    pub handled: bool,
    /// If `Some`, submit this text as a prompt to the agent session.
    pub prompt: Option<String>,
}

/// Dispatch a key to the active notepad.
///
/// Returns `KeyResult { handled: false, .. }` when the notepad is not open.
/// When `handled` is true the caller should set `dirty = true` and `continue`.
/// When `prompt` is `Some(t)` the caller should start a turn with `t`.
pub(crate) async fn key(
    notepad: &mut Option<NotepadView>,
    bash_rx: &mut Option<oneshot::Receiver<String>>,
    k: KeyEvent,
    keymap: &KeyBindings,
) -> KeyResult {
    if notepad.is_none() {
        return KeyResult {
            handled: false,
            prompt: None,
        };
    }

    // Toggle console panel visibility (Ctrl+Shift+T).
    if keymap.toggle_console.matches(&k) {
        if let Some(np) = notepad.as_mut() {
            np.console_hidden = !np.console_hidden;
            if np.console_hidden && np.focus == Focus::Console {
                np.focus = Focus::Editor;
            }
        }
        return KeyResult {
            handled: true,
            prompt: None,
        };
    }

    let outcome = crate::notepad::dispatch_key(notepad, k).await;
    let prompt = match outcome {
        NotepadOutcome::SubmitPrompt(text) => {
            if let Some(np) = notepad.as_mut() {
                np.console.set_running(true);
            }
            Some(text)
        }
        NotepadOutcome::RunBash(cmd) => {
            if let Some(np) = notepad.as_ref() {
                let workdir = np.workdir.clone();
                *bash_rx = Some(crate::notepad::console::submit::spawn_bash(&cmd, &workdir));
            }
            None
        }
        _ => None,
    };
    KeyResult {
        handled: true,
        prompt,
    }
}

/// Route a paste event into the notepad console (if active and in insert mode).
///
/// Returns `true` when the paste was consumed (caller should set `dirty`).
pub(crate) fn paste(notepad: &mut Option<NotepadView>, pasted: &str) -> bool {
    let np = match notepad.as_mut() {
        Some(v)
            if v.focus == Focus::Console && v.console.vim.mode == crate::vim::VimMode::Insert =>
        {
            v
        }
        _ => return false,
    };
    for c in pasted.chars() {
        if !c.is_control() {
            let (t, ci) =
                crate::composer::insert_char(&np.console.vim.text, np.console.vim.cursor, c);
            np.console.vim.text = t;
            np.console.vim.cursor = ci;
        }
    }
    true
}

/// Poll the background bash command from the notepad console.
///
/// Returns `true` when the console state changed (caller should set `dirty`).
pub(crate) fn poll_bash(
    notepad: &mut Option<NotepadView>,
    bash_rx: &mut Option<oneshot::Receiver<String>>,
) -> bool {
    use tokio::sync::oneshot::error::TryRecvError;
    let rx = match bash_rx.as_mut() {
        Some(r) => r,
        None => return false,
    };
    match rx.try_recv() {
        Ok(out) => {
            if let Some(np) = notepad.as_mut() {
                np.console.finish_bash(&out);
            }
            *bash_rx = None;
            true
        }
        Err(TryRecvError::Empty) => false,
        Err(TryRecvError::Closed) => {
            if let Some(np) = notepad.as_mut() {
                np.console.finish_bash("(command aborted)");
            }
            *bash_rx = None;
            true
        }
    }
}

/// Render the notepad frame if active.
///
/// Returns `Ok(true)` when the notepad was rendered (caller should skip the
/// normal `app_loop::render_frame`), `Ok(false)` otherwise.
pub(crate) fn try_render(terminal: &mut Term, notepad: &Option<NotepadView>) -> Result<bool> {
    if let Some(np) = notepad {
        crate::notepad::render_frame(terminal, np)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
