//! `!cmd` local bash-tool helpers (`push_bash_tool` / `finish_bash_tool`):
//! a single-call `ToolGroup` that starts in the Results state (output visible
//! while the command runs) and collapses once the command finishes.

use super::super::*;

#[test]
fn push_bash_tool_starts_in_results_state() {
    let mut v = ChatView::default();
    v.push_bash_tool("echo hi");
    match v.blocks.last() {
        Some(ChatBlock::ToolGroup { calls, state, .. }) => {
            assert_eq!(calls.len(), 1);
            assert!(
                matches!(state, ToolGroupState::Results),
                "local `!cmd` runs visible from the start (Results state)"
            );
            assert!(
                calls[0].id.starts_with("bash-"),
                "synthetic id: {:?}",
                calls[0].id
            );
        }
        other => panic!("expected ChatBlock::ToolGroup as last block, got {other:?}"),
    }
}

#[test]
fn finish_bash_tool_fills_output_and_collapses() {
    let mut v = ChatView::default();
    v.push_bash_tool("echo hi");
    v.finish_bash_tool("hello\nworld");

    match v.blocks.last() {
        Some(ChatBlock::ToolGroup { calls, state, .. }) => {
            let call = &calls[0];
            assert!(
                !call.output.is_empty(),
                "output must contain lines after finish_bash_tool"
            );
            let joined: String = call
                .output
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.clone())
                .collect();
            assert!(
                joined.contains("hello") && joined.contains("world"),
                "output must preserve both lines; got {joined:?}"
            );
            assert!(
                matches!(state, ToolGroupState::Collapsed),
                "group must collapse once the command finishes"
            );
            assert!(
                call.elapsed_ms.is_some(),
                "elapsed_ms must be recorded after finish_bash_tool"
            );
        }
        other => panic!("expected ChatBlock::ToolGroup as last block, got {other:?}"),
    }
}

#[test]
fn finish_bash_tool_aborted_message() {
    // When a command is aborted (e.g. user interrupt), the notepad layer
    // passes "(command aborted)" as the output. The call must surface that
    // text so the transcript explains why there is no real result.
    let mut v = ChatView::default();
    v.push_bash_tool("sleep 999");
    v.finish_bash_tool("(command aborted)");

    match v.blocks.last() {
        Some(ChatBlock::ToolGroup { calls, .. }) => {
            let joined: String = calls[0]
                .output
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.clone())
                .collect();
            assert!(
                joined.contains("aborted"),
                "aborted output must be visible in the call; got {joined:?}"
            );
        }
        other => panic!("expected ChatBlock::ToolGroup as last block, got {other:?}"),
    }
}
