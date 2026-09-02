//! Regression tests for duration/running rendering on replayed step groups.
//!
//! Replayed tool calls carry no wall-clock timing from the persisted
//! `Message`s. They must be marked finished (`elapsed_ms: Some(0)`) so the
//! group line shows neither a live timer nor the "running" spinner hint on
//! `--continue`/resume.

use super::replay::replay_one;
use crate::chat::{ChatBlock, ChatView};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use std::collections::HashMap;

/// Replaying an assistant ToolUse must mark the call done
/// (`elapsed_ms: Some(0)`) so the group line shows no running hint.
#[test]
fn replayed_tool_block_omits_duration_span() {
    let msg = Message {
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
        .find(|l| l.spans.iter().any(|s| s.content.contains("1 step")))
        .expect("group line should be present")
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();

    assert!(
        !group_line.contains("running"),
        "replayed group line must not show the running hint: {group_line}"
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

/// Replaying an assistant message carrying Reasoning blocks must restore them
/// as collapsed `ChatBlock::Thinking` blocks — so the `💭 Thinking` label
/// survives resume / compaction (mirrors the live ReasoningDelta path).
#[test]
fn replayed_reasoning_restored_as_thinking_block() {
    let msg = Message {
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

    let thinking = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Thinking { .. }))
        .expect("Reasoning block must be restored as a Thinking block");

    match thinking {
        ChatBlock::Thinking {
            text,
            collapsed,
            sealed,
        } => {
            assert_eq!(text, "think hard");
            assert!(*collapsed, "replayed thinking starts collapsed");
            assert!(*sealed, "replayed thinking is sealed (not streaming)");
        }
        _ => unreachable!(),
    }

    let thinking_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Thinking { .. }))
        .expect("thinking index");
    let assistant_idx = chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::Assistant { .. }))
        .expect("assistant text block must still be replayed");
    assert!(
        thinking_idx < assistant_idx,
        "replay must preserve the live Thinking-before-Say order"
    );
}
