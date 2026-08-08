//! Replay logic for reconstructing a [`ChatView`] from persisted messages and
//! subagent task records during session resume or context restore.
//!
//! Extracted from `session_ui.rs` to keep that module focused on UI-state
//! snapshot/restore. The public entry point is [`replay_into_chat`].

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::estimate_messages_for_display;
use opencoder_session::SessionEvent;
use opencoder_store::{Store, SubagentStatus, SubagentTaskRecord};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::chat::{short, summarize, ChatBlock, ChatView, TOOL_OUTPUT_LINES};
use crate::terminal_text::{sanitize_multiline, sanitize_single_line};
use crate::theme;

/// Replay a single persisted message into `chat`: reconstruct `Assistant` text and
/// `Tool` blocks (header from `ToolUse`, output from matching `ToolResult`),
/// mirroring the live `ChatView::apply` path for resumed/compacted sessions.
pub(super) fn replay_one(
    chat: &mut ChatView,
    msg: &Message,
    prefetched: &HashMap<String, Vec<u8>>,
) {
    match msg.role {
        Role::User => {
            // Synthetic user messages (plan->act handoff, compaction summaries,
            // pure-skill triggers) are internal — skip `user:` blocks on replay.
            // Steer/queue promotions are real user input and ARE rendered so the
            // user sees their queued/steered prompts after resume.
            if msg.synthetic {
                return;
            }
            let text: String = msg
                .blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let text = sanitize_multiline(&text).into_owned();
            if chat.first_prompt.is_none() {
                let t = text.trim();
                if !t.is_empty() {
                    chat.first_prompt = Some(t.to_string());
                }
            }
            let has_images = msg.blocks.iter().any(|b| b.as_image().is_some());
            if text.is_empty() && !has_images {
                return;
            }
            chat.push_marker(Line::from(Span::styled(
                "user:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            let rendered = crate::markdown::render(&text);
            if !rendered.is_empty() {
                chat.blocks.push(ChatBlock::Marker(rendered));
            }
            // Render any Image blocks inline after the text. Remote URLs are
            // resolved from the prefetched-bytes map (async-fetched above).
            for b in &msg.blocks {
                if let ContentBlock::Image { url, .. } = b {
                    let filename = sanitize_single_line(&crate::image_util::extract_filename(url))
                        .into_owned();
                    let rendered_img = crate::image_render::render_image_from_url(url, prefetched);
                    chat.blocks.push(ChatBlock::Image {
                        filename,
                        rendered: rendered_img,
                    });
                }
            }
            chat.push_marker(Line::from(""));
        }
        Role::Assistant => {
            let text: String = msg
                .blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let text = sanitize_multiline(&text).into_owned();
            if !text.is_empty() {
                let rendered = crate::markdown::render(&text);
                chat.blocks.push(ChatBlock::Assistant {
                    raw: text,
                    rendered,
                    done: true,
                });
            }
            // Restore reasoning blocks as collapsed Thinking blocks so the
            // `💭 Thinking` label survives resume / compaction — mirroring the
            // live `ChatView::apply` ReasoningDelta path.
            for b in &msg.blocks {
                if let ContentBlock::Reasoning { text } = b {
                    chat.blocks.push(ChatBlock::Thinking {
                        text: sanitize_multiline(text).into_owned(),
                        collapsed: true,
                        sealed: true,
                    });
                }
            }
            for b in &msg.blocks {
                if let ContentBlock::ToolUse { id, name, input } = b {
                    if name == "task" {
                        continue;
                    }
                    chat.blocks.push(ChatBlock::Tool {
                        id: id.clone(),
                        header: Line::from(vec![
                            Span::styled(
                                format!("\u{25b8} {} ", sanitize_single_line(name)),
                                Style::default()
                                    .fg(theme::accent())
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(summarize(input), Style::default().fg(theme::muted())),
                        ]),
                        output: Vec::new(),
                        collapsed: true,
                        started_at_ms: 0,
                        elapsed_ms: Some(0),
                    });
                }
            }
        }
        Role::Tool => {
            for b in &msg.blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    images,
                    ..
                } = b
                {
                    let color = if *is_error {
                        theme::err_color()
                    } else {
                        theme::muted()
                    };
                    let clean_content = sanitize_multiline(content);
                    let out: Vec<Line<'static>> = clean_content
                        .lines()
                        .take(TOOL_OUTPUT_LINES)
                        .map(|l| {
                            Line::from(Span::styled(format!("  {l}"), Style::default().fg(color)))
                        })
                        .collect();
                    if let Some(ChatBlock::Tool { output: o, .. }) = chat
                        .blocks
                        .iter_mut()
                        .rev()
                        .find(|blk| {
                            matches!(blk, ChatBlock::Tool { id: bid, .. } if bid == tool_use_id)
                        }) {
                        o.extend(out);
                    } else {
                        // Skip fallback for "task" tools — their output is
                        // shown via the Subagent block, not a Tool block.
                        let has_subagent = chat.blocks.iter().any(|b| {
                            matches!(b, ChatBlock::Subagent { id: bid, .. } if bid == tool_use_id)
                        });
                        if !has_subagent {
                            chat.blocks.push(ChatBlock::Tool {
                                id: tool_use_id.clone(),
                                header: Line::from(Span::styled(
                                    "\u{25b8} (output)",
                                    Style::default().fg(theme::accent()),
                                )),
                                output: out,
                                collapsed: true,
                                started_at_ms: 0,
                                elapsed_ms: Some(0),
                            });
                        }
                    }
                    // Render tool-returned images inline after the text output.
                    for url in images {
                        let filename =
                            sanitize_single_line(&crate::image_util::extract_filename(url))
                                .into_owned();
                        let rendered_img =
                            crate::image_render::render_image_from_url(url, prefetched);
                        chat.blocks.push(ChatBlock::Image {
                            filename,
                            rendered: rendered_img,
                        });
                    }
                }
            }
        }
        Role::System => {}
    }
}

/// Build a fresh `ChatView` for a resumed session by replaying stored messages
/// and reconstructing subagent blocks from persisted `subagent_tasks` records.
pub async fn replay_into_chat(
    agent_name: &str,
    messages: &[Message],
    store: &Arc<dyn Store>,
    session_id: &str,
) -> ChatView {
    let mut chat = ChatView {
        agent: sanitize_single_line(agent_name).into_owned(),
        ..Default::default()
    };

    // Plan→act handoff card. The clear-context boundary (/act_clear_context)
    // also persists an internal sentinel here — skip it so the raw marker
    // never reaches the UI.
    if let Ok(Some(meta)) = store.get_session(session_id).await {
        if let Some(plan) = meta
            .handoff_plan
            .as_deref()
            .filter(|p| !opencoder_session::is_clear_context_handoff(p))
        {
            let clean_plan = sanitize_multiline(plan);
            let rendered = crate::markdown::render(&clean_plan);
            if !rendered.is_empty() {
                chat.blocks.push(ChatBlock::Plan {
                    rendered,
                    raw: clean_plan.into_owned(),
                });
            }
        }
    }

    let tasks = store
        .list_subagent_tasks(session_id)
        .await
        .unwrap_or_default();
    let mut tasks_by_parent: HashMap<String, Vec<SubagentTaskRecord>> = HashMap::new();
    let mut orphan_tasks: Vec<SubagentTaskRecord> = Vec::new();
    for task in tasks {
        match &task.parent_message_id {
            Some(mid) => {
                tasks_by_parent.entry(mid.clone()).or_default().push(task);
            }
            None => {
                orphan_tasks.push(task);
            }
        }
    }
    for group in tasks_by_parent.values_mut() {
        group.sort_by_key(|t| t.started_at);
    }
    orphan_tasks.sort_by_key(|t| t.started_at);

    // Pre-fetch HTTP image URLs so remote images render during synchronous
    // replay. Data URIs are handled inline by `render_image_from_url`.
    let prefetched = prefetch_image_bytes(messages).await;

    for msg in messages {
        replay_one(&mut chat, msg, &prefetched);
        // Interleave child blocks under their parent assistant message.
        if msg.role == Role::Assistant {
            if let Some(task_list) = tasks_by_parent.remove(&msg.id) {
                for task in task_list {
                    let block = build_subagent_block(&task, store).await;
                    chat.blocks.push(block);
                }
            }
        }
    }

    for task in orphan_tasks {
        let block = build_subagent_block(&task, store).await;
        chat.blocks.push(block);
    }

    // Full transcript token count for ctx% (system prompt added at render).
    chat.context_used = estimate_messages_for_display(messages) as u64;
    chat
}

/// Reconstruct a `ChatBlock::Subagent` from a persisted `SubagentTaskRecord`,
/// including rebuilding the child `ChatView` from stored events.
pub(super) async fn build_subagent_block(
    task: &SubagentTaskRecord,
    store: &Arc<dyn Store>,
) -> ChatBlock {
    let (done, ok, cancelled, summary) = match task.status {
        SubagentStatus::Completed => (
            true,
            task.ok.unwrap_or(true),
            false,
            task.result.clone().unwrap_or_default(),
        ),
        SubagentStatus::Failed => (true, false, false, task.result.clone().unwrap_or_default()),
        SubagentStatus::Cancelled => (true, false, true, "(cancelled)".to_string()),
        SubagentStatus::Running | SubagentStatus::Unknown => {
            // Interrupted during resume — display as done/failed with a marker.
            (true, false, false, "(interrupted)".to_string())
        }
    };

    let view = reconstruct_child_view(&task.child_session_id, &task.agent, store).await;

    ChatBlock::Subagent {
        id: task.task_id.clone(),
        child_session_id: task.child_session_id.clone(),
        kind: sanitize_single_line(&task.agent).into_owned(),
        prompt: short(&task.prompt, 90),
        view,
        done,
        ok,
        cancelled,
        summary: sanitize_multiline(&summary).into_owned(),
        started_at_ms: task.started_at,
        elapsed_ms: task
            .completed_at
            .map(|c| ((c - task.started_at).max(0)) as u64),
    }
}

/// Rebuild a child `ChatView` from persisted events (primary) or messages
/// (fallback) under the child session id.
pub(super) async fn reconstruct_child_view(
    child_session_id: &str,
    agent_name: &str,
    store: &Arc<dyn Store>,
) -> ChatView {
    // Primary: replay persisted events.
    let events = store
        .events_after(child_session_id, 0)
        .await
        .unwrap_or_default();
    if !events.is_empty() {
        let mut view = ChatView {
            agent: sanitize_single_line(agent_name).into_owned(),
            ..Default::default()
        };
        for rec in &events {
            if let Some(ev) = deserialize_event(&rec.payload) {
                view.apply(&ev);
            }
        }
        // Events may be incomplete; compute context_used from the full
        // message list for an accurate token estimate.
        let child_msgs = store
            .load_messages(child_session_id)
            .await
            .unwrap_or_default();
        view.context_used = estimate_messages_for_display(&child_msgs) as u64;
        return view;
    }

    // Fallback: replay messages.
    tracing::debug!(
        child_session_id,
        "no persisted events for subagent child, falling back to messages"
    );
    let messages = store
        .load_messages(child_session_id)
        .await
        .unwrap_or_default();
    if messages.is_empty() {
        tracing::debug!(
            child_session_id,
            "no events or messages for subagent child session"
        );
    }
    replay_messages(agent_name, &messages)
}

/// Deserialize a `SessionEvent` from a stored event payload.
/// Child events are double-encoded: `Value::String(json_string)`.
pub(super) fn deserialize_event(payload: &serde_json::Value) -> Option<SessionEvent> {
    match payload {
        serde_json::Value::String(s) => serde_json::from_str::<SessionEvent>(s)
            .map_err(|e| tracing::warn!(error = %e, "failed to deserialize string event payload"))
            .ok(),
        other => serde_json::from_value::<SessionEvent>(other.clone())
            .map_err(|e| tracing::warn!(error = %e, "failed to deserialize json event payload"))
            .ok(),
    }
}

/// Collect and asynchronously fetch all HTTP(S) image URLs from `messages`
/// so they are available during synchronous `replay_one`. Data URIs are
/// skipped (decoded in-process). Failed fetches are silently dropped —
/// the corresponding images will show a placeholder.
pub(super) async fn prefetch_image_bytes(messages: &[Message]) -> HashMap<String, Vec<u8>> {
    let mut urls: Vec<String> = Vec::new();
    for msg in messages {
        for b in &msg.blocks {
            if let ContentBlock::Image { url, .. } = b {
                if url.starts_with("http://") || url.starts_with("https://") {
                    urls.push(url.clone());
                }
            }
            if let ContentBlock::ToolResult { images, .. } = b {
                for url in images {
                    if url.starts_with("http://") || url.starts_with("https://") {
                        urls.push(url.clone());
                    }
                }
            }
        }
    }
    urls.sort();
    urls.dedup();

    let mut map = HashMap::new();
    for url in urls {
        if let Some(bytes) = crate::image_render::fetch_image_bytes(&url).await {
            map.insert(url, bytes);
        }
    }
    map
}

/// Text-only message replay (no subagent reconstruction). Used as a fallback
/// for child views without persisted events, and by tests.
pub fn replay_messages(agent_name: &str, messages: &[Message]) -> ChatView {
    let empty = HashMap::new();
    let mut chat = ChatView {
        agent: sanitize_single_line(agent_name).into_owned(),
        ..Default::default()
    };
    for msg in messages {
        replay_one(&mut chat, msg, &empty);
    }
    chat.context_used = estimate_messages_for_display(messages) as u64;
    chat
}
