//! Regression tests for duration/running rendering on replayed step groups.
//!
//! Replayed tool calls carry no wall-clock timing from the persisted
//! `Message`s. They must be marked finished (`elapsed_ms: Some(0)`) so the
//! group marker shows neither a live timer nor the "running" spinner hint on
//! `--continue`/resume.

use super::replay::replay_one;
use crate::chat::{ChatBlock, ChatView};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use std::collections::HashMap;

/// Replaying an assistant ToolUse must mark the call done
/// (`elapsed_ms: Some(0)`) so the group marker shows no running hint.
#[test]
fn replayed_tool_block_omits_duration_span() {
    let msg = Message {
        display: None,
        id: "a1".into(),
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text {
                text: "running bash".into(),
            },
            ContentBlock::ToolUse {
                id: "t1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
        ],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let mut chat = ChatView::default();
    replay_one(&mut chat, &msg, &HashMap::new());

    let group = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .expect("should have a StepGroup block");

    match group {
        ChatBlock::StepGroup { steps, .. } => {
            assert_eq!(steps[0].calls[0].elapsed_ms, Some(0));
        }
        _ => unreachable!(),
    }

    // Flatten with an epoch-scale now_ms. A not-finished call would show the
    // running hint (and, before the group rework, an epoch-scale live timer).
    let now_ms = 1_700_000_000_000_i64;
    let garbage = crate::fmt::format_run_duration(now_ms as u64);
    let lines = chat.flatten_with(0, now_ms);
    // Scope the assertions to the GROUP LINE itself — the assistant body in
    // this fixture legitimately contains the word "running".
    let group_line: String = lines
        .iter()
        .find(|l| l.spans.iter().any(|s| s.content.contains("1 Step")))
        .expect("group marker should be present")
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();

    assert!(
        !group_line.contains("running"),
        "replayed group marker must not show the running hint: {group_line}"
    );
    assert!(
        !group_line.contains(&garbage),
        "replayed group must not render garbage duration '{garbage}'"
    );
}

/// The fallback orphan-tool-result path (Role::Tool with no matching ToolUse
/// block) must also produce a finished call (no running hint).
#[test]
fn replayed_orphan_tool_result_omits_duration_span() {
    let msg = Message {
        display: None,
        id: "tr1".into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: "orphan".into(),
            content: "done".into(),
            is_error: false,
            images: vec![],
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let mut chat = ChatView::default();
    replay_one(&mut chat, &msg, &HashMap::new());

    let group = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .expect("should have a fallback StepGroup block");

    match group {
        ChatBlock::StepGroup { steps, .. } => {
            assert_eq!(steps[0].calls[0].elapsed_ms, Some(0));
        }
        _ => unreachable!(),
    }
}

/// Replaying an assistant message carrying Reasoning blocks must fold them
/// into the ladder as a rendered, call-less step ABOVE the Say — mirroring
/// the live path (`1 Turn = n Steps + Say`); no top-level `Thinking` block
/// survives resume / compaction.
#[test]
fn replayed_reasoning_folds_into_the_ladder_step() {
    let msg = Message {
        display: None,
        id: "r1".into(),
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Reasoning {
                text: "think hard".into(),
            },
            ContentBlock::Text {
                text: "final answer".into(),
            },
        ],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let mut chat = ChatView::default();
    replay_one(&mut chat, &msg, &HashMap::new());

    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Thinking { .. })),
        "replayed reasoning lives inside the ladder, not a top-level block"
    );
    let group_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::StepGroup { .. }))
        .expect("Reasoning block must fold into the ladder's step");
    match &chat.blocks[group_idx] {
        ChatBlock::StepGroup { steps, .. } => {
            assert_eq!(steps.len(), 1);
            assert_eq!(steps[0].thinking_raw, "think hard");
            let rendered: String = steps[0]
                .thinking
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.clone())
                .collect();
            assert!(
                rendered.contains("think hard"),
                "replayed step thinking is rendered eagerly: {rendered:?}"
            );
            assert!(steps[0].calls.is_empty());
        }
        _ => unreachable!(),
    }

    let assistant_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Assistant { .. }))
        .expect("assistant text block must still be replayed");
    assert!(
        group_idx < assistant_idx,
        "replay must preserve the live Thinking-before-Say order"
    );
}
