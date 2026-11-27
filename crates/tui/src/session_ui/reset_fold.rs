//! Store-free nested reset fold (subagent child / sidecar views).
//!
//! A nested view receives its `TranscriptReset` as an event inside
//! `ChatView::apply`, where no store handle exists to rebuild subagent
//! records or prefetch images. [`rebuild_view_from_messages`] mirrors
//! [`super::replay::rebuild_after_reset`]: the same fold — `[Compaction,
//! retained tail, continued run]`, ONE collapsed summary block at the head of
//! the transcript instead of the whole pre-compaction history plus every
//! summary it ever produced — and the same cross-reset carry-over. The
//! parent's own reset never goes through here: the app loop intercepts a
//! local one for the store-aware off-loop rebuild and replays a remote one
//! inline.
//!
//! Grandchild `Subagent` cards exist only in the view being folded, so they
//! are carried across instead of being rebuilt from records: every card is
//! re-attached under the assistant message that spawned it (the task id IS
//! its `task` tool_use id — the identity `replay_one` keys on too) and, when
//! that message was folded away, kept by appending it like the store replay's
//! orphan tasks — so a running grandchild stays visible AND routable
//! (`SubagentChild` routing looks the card up by task id). A card's token
//! spend already sits inside the carried-over `tokens_total`, so
//! re-attaching never re-adds it (unlike `replay::push_subagent_block`).

use std::collections::HashMap;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::estimate_messages_for_display;

use super::replay::{
    compaction_summary_of, has_compaction_summary, push_compaction_summary_block, replay_one,
};
use crate::chat::{coalesce_steps, ChatBlock, ChatView};
use crate::terminal_text::sanitize_single_line;

/// Whether `msg` is the assistant message that spawned subagent task
/// `task_id`: the task id is its `task` tool_use id.
fn spawned(msg: &Message, task_id: &str) -> bool {
    msg.role == Role::Assistant
        && msg
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == task_id))
}

/// Move the carried cards spawned by `msg` to the end of the folded blocks,
/// preserving their live order. Plain push — their spend is already inside
/// the view's carried-over `tokens_total`.
fn attach_spawned_by(chat: &mut ChatView, msg: &Message, carried: &mut Vec<ChatBlock>) {
    let (here, rest): (Vec<ChatBlock>, Vec<ChatBlock>) = std::mem::take(carried)
        .into_iter()
        .partition(|b| matches!(b, ChatBlock::Subagent { id, .. } if spawned(msg, id)));
    chat.blocks.extend(here);
    *carried = rest;
}

/// Text-only fold of `messages` that re-attaches `carried` grandchild cards —
/// the store-free counterpart of `replay_into_chat`'s `tasks_by_parent`
/// interleave (same position rule: right after the parent assistant message).
pub(super) fn replay_messages_with(
    agent_name: &str,
    messages: &[Message],
    mut carried: Vec<ChatBlock>,
) -> ChatView {
    let empty = HashMap::new();
    let mut chat = ChatView {
        agent: sanitize_single_line(agent_name).into_owned(),
        ..Default::default()
    };
    for msg in messages {
        if let Some(summary) = compaction_summary_of(msg) {
            push_compaction_summary_block(&mut chat, &summary);
            continue;
        }
        replay_one(&mut chat, msg, &empty);
        attach_spawned_by(&mut chat, msg, &mut carried);
    }
    // The store replay drops a record whose parent message left the
    // transcript (nothing to hang it on); the live card is kept instead.
    chat.blocks.extend(carried);
    // Same fold as `replay_into_chat` — one shared step-shape guarantee.
    coalesce_steps(&mut chat.blocks);
    chat.context_used = estimate_messages_for_display(messages) as u64;
    chat
}

/// Store-free mirror of [`super::replay::rebuild_after_reset`] for NESTED
/// views (subagent child, sidecar): same fold and the same cross-reset
/// carry-over. `pending_turn_echo` follows the same rule as the parent:
/// dropped on a compacting rebuild, kept otherwise. Grandchild cards AND
/// their counters are carried over (see the module doc); lifetime tokens are
/// floored so `[tok cost]` never regresses.
pub fn rebuild_view_from_messages(chat: &mut ChatView, msgs: &[Message]) {
    let agent = chat.agent.clone();
    let carried: Vec<ChatBlock> = chat
        .blocks
        .iter()
        .filter(|b| matches!(b, ChatBlock::Subagent { .. }))
        .cloned()
        .collect();
    let saved_annotation_text = chat.annotation_text.clone();
    let saved_submitted = chat.submitted;
    let saved_first_prompt = chat.first_prompt.clone();
    let saved_tokens_total = chat.tokens_total;
    let saved_turn_echo = chat.pending_turn_echo.clone();
    let saved_running = chat.subagents_running;
    let saved_total = chat.subagents_total;
    let compacting = has_compaction_summary(msgs);
    *chat = replay_messages_with(&agent, msgs, carried);
    chat.tokens_total = chat.tokens_total.max(saved_tokens_total);
    chat.annotation_text = saved_annotation_text;
    chat.submitted = saved_submitted;
    chat.first_prompt = saved_first_prompt;
    chat.pending_turn_echo = if compacting { None } else { saved_turn_echo };
    chat.subagents_running = saved_running;
    chat.subagents_total = saved_total;
    // A compaction rebuild owns the whole transcript: reliable completion
    // repair must never target pre-reset blocks.
    chat.turn_block_start = chat.blocks.len();
}
