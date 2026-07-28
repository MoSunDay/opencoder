//! Plan-edit popup key-dispatch regression tests.
//!
//! Guards the `app_loop::handle_plan_edit_key` *integration* boundary: the
//! take()/restore logic that decides whether the plan-edit popup stays open.
//! The vim-level unit tests (`vim/insert.rs`, `plan_edit.rs`) only prove Ctrl+C
//! yields `VimAction::Continue`; if the take()/restore branches here were ever
//! swapped, the popup would silently close while every lower-level test stayed
//! green.
//!
//! Split out of `app_loop_tests.rs` to keep that file under the 800-line cap,
//! mirroring the `#[path]` convention of `app_loop_session_only_tests.rs`.

use super::super::*;
use crate::chat::ChatView;
use crate::plan_edit::PlanEdit;
use crate::worker::UiCmd;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// An active plan-edit popup seeded with editable text.
fn open_plan_edit() -> Option<PlanEdit> {
    Some(PlanEdit::new("edit me".to_string()))
}

fn char_key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
}
fn esc_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())
}
fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
}
/// Ctrl+C as the canonical `Char('c') + CONTROL` chord (Kitty-keyboard /
/// terminals that report modifiers).
fn ctrl_c_chord() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
}
/// Ctrl+C as the raw ETX byte (`\u{3}`) — the form raw mode actually delivers
/// on most terminals, since ISIG is disabled so the kernel does not translate
/// it into SIGINT.
fn ctrl_c_etx() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('\u{3}'), KeyModifiers::empty())
}

/// Ctrl+C (modifier-chord form) must keep the popup open: drop to Normal, no
/// exit, no persist, no UiCmd.
#[tokio::test]
async fn plan_edit_ctrl_c_chord_keeps_popup_open() {
    let mut plan_edit = open_plan_edit();
    let mut chat = ChatView::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(8);

    let flow = handle_plan_edit_key(&mut plan_edit, ctrl_c_chord(), &mut chat, &cmd_tx, 80).await;

    assert!(matches!(flow, LoopFlow::Redraw));
    assert!(
        plan_edit.is_some(),
        "Ctrl+C must NOT close the plan-edit popup"
    );
    assert_eq!(
        plan_edit.as_ref().unwrap().mode_label(),
        "NORMAL",
        "Ctrl+C drops to Normal but keeps the editor open"
    );
    assert!(
        cmd_rx.try_recv().is_err(),
        "no UiCmd (EditPlan/Quit) should fire when the popup stays open"
    );
}

/// The raw ETX form of Ctrl+C — what raw-mode terminals actually deliver —
/// must also keep the popup open.
#[tokio::test]
async fn plan_edit_ctrl_c_etx_keeps_popup_open() {
    let mut plan_edit = open_plan_edit();
    let mut chat = ChatView::default();
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(8);

    handle_plan_edit_key(&mut plan_edit, ctrl_c_etx(), &mut chat, &cmd_tx, 80).await;

    assert!(
        plan_edit.is_some(),
        "raw ETX must NOT close the plan-edit popup"
    );
    assert_eq!(plan_edit.as_ref().unwrap().mode_label(), "NORMAL");
}

/// Esc is the canonical "leave Insert, stay in editor" key — same family as
/// Ctrl+C. Guards that take()/restore treats every `Continue` the same, not
/// just Ctrl+C specifically.
#[tokio::test]
async fn plan_edit_esc_keeps_popup_open() {
    let mut plan_edit = open_plan_edit();
    let mut chat = ChatView::default();
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(8);

    handle_plan_edit_key(&mut plan_edit, esc_key(), &mut chat, &cmd_tx, 80).await;

    assert!(
        plan_edit.is_some(),
        "Esc must NOT close the plan-edit popup"
    );
    assert_eq!(plan_edit.as_ref().unwrap().mode_label(), "NORMAL");
}

/// Contrast / positive control: only `:wq` (or `:q`/`:q!`) in Command mode
/// closes the popup, and a modified buffer is persisted + dispatched as
/// `UiCmd::EditPlan`. Pins the exit branch so the take()/restore logic is
/// covered in both directions.
#[tokio::test]
async fn plan_edit_wq_exits_and_persists_modified_text() {
    let mut plan_edit = open_plan_edit();
    let mut chat = ChatView::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(8);

    // PlanEdit opens in Normal; 'A' -> Insert at end, append 'z', Esc -> Normal,
    // ':' -> Command, "wq", Enter -> Exit (persist).
    handle_plan_edit_key(&mut plan_edit, char_key('A'), &mut chat, &cmd_tx, 80).await;
    handle_plan_edit_key(&mut plan_edit, char_key('z'), &mut chat, &cmd_tx, 80).await;
    handle_plan_edit_key(&mut plan_edit, esc_key(), &mut chat, &cmd_tx, 80).await;
    handle_plan_edit_key(&mut plan_edit, char_key(':'), &mut chat, &cmd_tx, 80).await;
    handle_plan_edit_key(&mut plan_edit, char_key('w'), &mut chat, &cmd_tx, 80).await;
    handle_plan_edit_key(&mut plan_edit, char_key('q'), &mut chat, &cmd_tx, 80).await;
    let flow = handle_plan_edit_key(&mut plan_edit, enter_key(), &mut chat, &cmd_tx, 80).await;

    assert!(matches!(flow, LoopFlow::Redraw));
    assert!(plan_edit.is_none(), ":wq must close the plan-edit popup");
    match cmd_rx.try_recv() {
        Ok(UiCmd::EditPlan(text)) => {
            assert_eq!(text, "edit mez", ":wq must persist the modified buffer");
        }
        Ok(_) => panic!("expected UiCmd::EditPlan, got a different variant"),
        Err(e) => panic!("expected UiCmd::EditPlan, but channel recv failed: {e:?}"),
    }
    assert!(
        cmd_rx.try_recv().is_err(),
        "exactly one UiCmd should fire on :wq"
    );
}
