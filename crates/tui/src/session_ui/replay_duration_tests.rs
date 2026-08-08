//! Regression tests for duration rendering on replayed Tool blocks.
//!
//! Replayed Tool blocks carry no wall-clock timing from the persisted
//! `Message`s. They must NOT enter the "running" branch of
//! `push_duration_span` (which would compute `now - 0 = epoch-ms` and
//! render a garbage timer on `--continue`/resume). Setting
//! `elapsed_ms: Some(0)` hits the sub-1s guard so no span is pushed.

use super::replay::replay_one;
use crate::chat::{ChatBlock, ChatView};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use std::collections::HashMap;

/// Replaying an assistant ToolUse must mark the Tool block done
/// (`elapsed_ms: Some(0)`) so it does NOT render an epoch-scale live timer.
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

    let tool = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Tool { .. }))
        .expect("should have a Tool block");

    match tool {
        ChatBlock::Tool { elapsed_ms, .. } => {
            assert_eq!(*elapsed_ms, Some(0));
        }
        _ => unreachable!(),
    }

    // Flatten with an epoch-scale now_ms. Before the fix `elapsed_ms: None`
    // would compute live = now - 0 and push format_run_duration(epoch_ms).
    let now_ms = 1_700_000_000_000_i64;
    let garbage = crate::fmt::format_run_duration(now_ms as u64);
    let lines = chat.flatten_with(0, now_ms);
    let rendered: String = lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect::<String>();

    assert!(rendered.contains("bash"), "tool header should be present");
    assert!(
        !rendered.contains(&garbage),
        "replayed Tool must not render garbage duration '{garbage}'"
    );
}

/// The fallback orphan-tool-result path (Role::Tool with no matching ToolUse
/// block) must also omit a duration span.
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

    let tool = chat
        .blocks
        .iter()
        .find(|b| matches!(b, ChatBlock::Tool { .. }))
        .expect("should have a fallback Tool block");

    match tool {
        ChatBlock::Tool { elapsed_ms, .. } => {
            assert_eq!(*elapsed_ms, Some(0));
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

    // The assistant text must still be present after the thinking block.
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::Assistant { .. })),
        "assistant text block must still be replayed"
    );
}
