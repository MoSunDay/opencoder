//! `/clear_context` popup dispatch: submits the control-command prompt when
//! idle, refused by the busy gate while running. (`ClearContext` keeps the
//! active agent — the runner emits only `TranscriptReset` + `Done`.)
use super::*;

/// `/clear_context` from idle submits the prompt verbatim.
#[tokio::test]
async fn slash_clear_context_from_idle_submits_prompt() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut menu = menu_for("clear");
    let (flow, mut cmd_rx, running) = dispatch_popup(&mut menu, &mut chat, false, "act").await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "the clear-context turn starts immediately from idle");
    match drain_cmd(&mut cmd_rx) {
        UiCmd::Prompt(text, _) => assert_eq!(text, "/clear_context"),
        other => panic!("expected Prompt(/clear_context), got {other:?}"),
    }
}

/// `/clear_context` while running is a no-op (same gate).
#[tokio::test]
async fn slash_clear_context_while_running_is_noop() {
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    let mut menu = menu_for("clear");
    let (flow, mut cmd_rx, running) = dispatch_popup(&mut menu, &mut chat, true, "act").await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(running, "running must stay true (turn still active)");
    assert!(
        cmd_rx.try_recv().is_err(),
        "no command should be sent while running"
    );
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Marker(lines)
            if lines.iter().any(|l| l.to_string().contains("busy")))),
        "a [switch] busy marker must be pushed; blocks: {:?}",
        chat.blocks
    );
}

/// Shift+Tab and typing the command are ONE path: the key handler emits
/// `Submit(CLEAR_CONTEXT_CMD [+ draft])`, which parses to
/// [`SlashAction::ClearContext`] exactly like the typed spellings, and the
/// canonical string round-trips through `control_cmd_string`.
#[test]
fn backtab_and_typed_clear_context_are_one_path() {
    use crate::command::{control_cmd_string, parse, SlashAction};
    use crate::key_handler::CLEAR_CONTEXT_CMD;

    // Every spelling the UI can produce parses to the same action.
    assert_eq!(parse(CLEAR_CONTEXT_CMD), Some(SlashAction::ClearContext));
    assert_eq!(parse("/clear_context"), Some(SlashAction::ClearContext));
    assert_eq!(parse("/act_clear_context"), Some(SlashAction::ClearContext));

    // The canonical submitted text is the runner's ClearContext control
    // command and round-trips.
    assert_eq!(
        control_cmd_string(&SlashAction::ClearContext),
        Some(CLEAR_CONTEXT_CMD)
    );

    // A Shift+Tab with a draft forwards it as the compound rest of the SAME
    // command — equivalent to typing "/clear_context <draft>".
    let typed = format!("{CLEAR_CONTEXT_CMD} finish the summary");
    let (cmd, rest) =
        opencoder_session::split_control_prefix(&typed).expect("compound must parse");
    assert!(matches!(cmd, opencoder_session::ControlCmd::ClearContext));
    assert_eq!(rest, Some("finish the summary".into()));
}
