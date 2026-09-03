//! `!cmd` local bash-tool helpers (`push_bash_tool` / `finish_bash_tool`):
//! a single-call `StepGroup` that starts fully expanded through every
//! ladder level (group → step → calls list → output, so the output is
//! visible while the command runs) and collapses back to the closed group
//! row once the command finishes.

use super::super::*;

#[test]
fn push_bash_tool_starts_in_results_state() {
    let mut v = ChatView::default();
    v.push_bash_tool("echo hi");
    match v.blocks.last() {
        Some(ChatBlock::StepGroup { steps, open, .. }) => {
            assert_eq!(steps.len(), 1);
            assert!(
                *open && steps[0].open && steps[0].calls_open && steps[0].calls[0].expanded,
                "local `!cmd` runs visible from the start (whole ladder expanded)"
            );
            assert!(
                steps[0].calls[0].id.starts_with("bash-"),
                "synthetic id: {:?}",
                steps[0].calls[0].id
            );
        }
        other => panic!("expected ChatBlock::StepGroup as last block, got {other:?}"),
    }
}

#[test]
fn finish_bash_tool_fills_output_and_collapses() {
    let mut v = ChatView::default();
    v.push_bash_tool("echo hi");
    v.finish_bash_tool("hello\nworld");

    match v.blocks.last() {
        Some(ChatBlock::StepGroup { steps, open, .. }) => {
            let call = &steps[0].calls[0];
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
                !*open && !steps[0].open && !steps[0].calls_open && !call.expanded,
                "every ladder level must collapse once the command finishes"
            );
            assert!(
                call.elapsed_ms.is_some(),
                "elapsed_ms must be recorded after finish_bash_tool"
            );
        }
        other => panic!("expected ChatBlock::StepGroup as last block, got {other:?}"),
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
        Some(ChatBlock::StepGroup { steps, .. }) => {
            let joined: String = steps[0].calls[0]
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
        other => panic!("expected ChatBlock::StepGroup as last block, got {other:?}"),
    }
}
