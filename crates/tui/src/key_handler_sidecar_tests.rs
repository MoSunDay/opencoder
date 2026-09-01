//! `/sidecar` key routing tests: the sidecar intercept must never leak a
//! question into the parent's steer/queue/submit paths, and a focused
//! sidecar box turns Enter into a follow-up while Tab stays rejected.

use super::*;

/// Parameterised `handle_key` press for the sidecar scenarios.
#[allow(clippy::too_many_arguments)]
fn press(code: KeyCode, text: &str, running: bool, sidecar_focused: bool) -> (KeyAction, String) {
    let mut input = text.to_string();
    let mut cursor = input.chars().count();
    let mut hist_idx = None;
    let mut scroll = 0;
    let mut follow = true;
    let mut last_esc = None;
    let mut skill_menu = None;
    let mut undo_state = crate::undo::init(&input, cursor);
    let mut queue_scroll = 0;
    let mut file_menu = None;
    let action = handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
        &mut input,
        &mut cursor,
        &[],
        &mut hist_idx,
        running,
        "act",
        &mut scroll,
        &mut follow,
        &mut last_esc,
        &mut skill_menu,
        80,
        2,
        false,
        sidecar_focused,
        false,
        &mut undo_state,
        &mut queue_scroll,
        &mut file_menu,
        Path::new("."),
    );
    (action, input)
}

/// Idle `/sidecar <question>` intercepts into `SidecarAsk` with the trimmed
/// question — never a `Submit` — and the composer is cleared.
#[test]
fn idle_sidecar_question_routes_to_the_sidecar_actor() {
    let (action, input) = press(KeyCode::Enter, "/sidecar 当前进度?", false, false);
    assert!(
        matches!(action, KeyAction::SidecarAsk(ref q) if q == "当前进度?"),
        "expected SidecarAsk, got {action:?}"
    );
    assert!(
        input.is_empty(),
        "the composer must be cleared after the ask"
    );
}

/// A follow-up typed INSIDE the focused sidecar box is a sidecar question —
/// even plain text without the `/sidecar` prefix.
#[test]
fn focused_sidecar_enter_is_always_a_follow_up() {
    let (action, input) = press(KeyCode::Enter, "那这个文件呢?", true, true);
    assert!(
        matches!(action, KeyAction::SidecarAsk(ref q) if q == "那这个文件呢?"),
        "expected SidecarAsk, got {action:?}"
    );
    assert!(input.is_empty());
}

/// Running + NOT focused: `/sidecar` must intercept before the running
/// path — the whole point is to ask WITHOUT steering the main task.
#[test]
fn running_sidecar_question_does_not_steer_the_parent() {
    let (action, input) = press(KeyCode::Enter, "/sidecar 慢吗?", true, false);
    assert!(
        matches!(action, KeyAction::SidecarAsk(ref q) if q == "慢吗?"),
        "running /sidecar must bypass Steer, got {action:?}"
    );
    assert!(input.is_empty());
}

/// `/sidecarX` is a different token: the word-boundary guard sends it down
/// the normal path (idle → Submit, running → Steer) untouched.
#[test]
fn sidecar_lookalike_prefix_is_not_intercepted() {
    let (idle_action, _) = press(KeyCode::Enter, "/sidecarX list", false, false);
    assert!(
        matches!(idle_action, KeyAction::Submit(ref t) if t == "/sidecarX list"),
        "idle look-alike must Submit, got {idle_action:?}"
    );
    let (running_action, _) = press(KeyCode::Enter, "/sidecarX list", true, false);
    assert!(
        matches!(running_action, KeyAction::Steer(ref t) if t == "/sidecarX list"),
        "running look-alike must Steer the parent, got {running_action:?}"
    );
}

/// Bare `/sidecar` (no question) is still a SidecarAsk — the app arm then
/// re-focuses an existing box or flashes the usage hint. Not a Submit.
#[test]
fn bare_sidecar_stays_on_the_sidecar_path() {
    let (action, _) = press(KeyCode::Enter, "/sidecar", false, false);
    assert!(matches!(action, KeyAction::SidecarAsk(ref q) if q.is_empty()));
}

/// Tab inside the focused sidecar box must never queue into the parent
/// session; the typed text is preserved for Enter (sidecar follow-up).
#[test]
fn focused_sidecar_tab_rejects_queue_and_keeps_draft() {
    let (action, input) = press(KeyCode::Tab, "排队不可能", true, true);
    assert!(
        matches!(action, KeyAction::QueueUnsupported),
        "expected QueueUnsupported, got {action:?}"
    );
    assert_eq!(input, "排队不可能", "the draft must survive for Enter");
}

/// A running focused sidecar routes the same as an idle one: the parent's
/// running state is invisible to the sidecar composer.
#[test]
fn focused_sidecar_turn_with_running_parent_still_asks() {
    let (action, _) = press(KeyCode::Enter, "/sidecar 还有别的吗", true, true);
    assert!(
        matches!(action, KeyAction::SidecarAsk(ref q) if q == "还有别的吗"),
        "focused sidecar must bypass Steer/Queue, got {action:?}"
    );
}
