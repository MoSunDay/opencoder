//! Shared helpers for the resume/replay integration suite.
//!
//! Kept in a separate module so each test file (`resume_replay.rs`,
//! `resume_cancelled_pending.rs`) stays within the per-file line budget
//! without duplicating the store/message fixtures.

#![allow(dead_code)] // each consuming test crate uses a different subset

use std::collections::HashSet;
use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{CompletedToolCall, LlmEvent, Usage};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

pub async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

pub fn config(model: &str) -> Config {
    Config {
        model: model.into(),
        ..Config::default()
    }
}

pub fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 3,
            total_tokens: 8,
            ..Default::default()
        }),
    }
}

pub fn session_meta(id: &str, agent: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some("test".into()),
        agent: Some(agent.into()),
        model: Some("m".into()),
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

/// A parent assistant turn that emits one or more `task` tool_use blocks.
pub fn parent_task_turn(task_ids: &[&str]) -> Message {
    let mut blocks: Vec<ContentBlock> = vec![ContentBlock::Text {
        text: "delegating".into(),
    }];
    for id in task_ids {
        blocks.push(ContentBlock::ToolUse {
            id: (*id).into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": "explore", "subagent_type": "explore"}),
        });
    }
    Message {
        id: "a1".into(),
        role: Role::Assistant,
        blocks,
        model: Some("m".into()),
        agent: Some("act".into()),
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

/// Collect the set of `tool_use` ids in `msgs` that have no matching
/// `tool_result` (i.e. would trigger dangling reconciliation).
pub fn dangling_tool_uses(msgs: &[Message]) -> Vec<String> {
    let answered: HashSet<&str> = msgs
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    msgs.iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } if !answered.contains(id.as_str()) => Some(id.clone()),
            _ => None,
        })
        .collect()
}
