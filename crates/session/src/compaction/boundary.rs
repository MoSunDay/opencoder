use opencoder_core::{ContentBlock, Message};
use std::collections::HashMap;

/// A tool request and all of its results belong on the same side of a
/// compaction boundary. Unanswered requests must stay in the retained tail.
pub(super) fn tool_safe_split(messages: &[Message], desired: usize) -> Option<usize> {
    let mut calls = HashMap::new();
    let mut results = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        for block in &message.blocks {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    calls.insert(id, index);
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    results.insert(tool_use_id, index);
                }
                _ => {}
            }
        }
    }
    (desired..messages.len())
        .chain((1..desired).rev())
        .find(|split| {
            *split > 0
                && calls.iter().all(|(id, start)| {
                    *start >= *split || results.get(id).is_some_and(|end| *end < *split)
                })
        })
}
