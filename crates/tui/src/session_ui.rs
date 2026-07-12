//! Per-session UI state snapshot — saved when switching sessions via `/task`
//! and restored when switching back, so chat history, scroll position, and
//! running status survive a session round-trip.

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_session::SessionEvent;
use opencoder_store::{Store, SubagentStatus, SubagentTaskRecord};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::chat::{short, ChatBlock, ChatView};

/// Snapshot of all session-specific TUI state. The `input`, `cursor_idx`,
/// `hist_idx`, and `last_esc` are intentionally NOT included — they are
/// interaction-local and reset cleanly on each switch.
#[derive(Clone, PartialEq)]
pub struct SessionUiState {
    pub running: bool,
    pub chat: ChatView,
    pub history: Vec<String>,
    pub scroll: u16,
    pub follow: bool,
    pub sys_tokens: u64,
    pub steer_items: Vec<String>,
    pub queue_items: Vec<(i64, String)>,
    pub active_skill: Option<String>,
    pub active_skill_body: Option<String>,
    pub agent_name: String,
}

impl SessionUiState {
    /// Create a fresh default state for a new session with the given agent.
    pub fn new(agent_name: String, sys_tokens: u64) -> Self {
        SessionUiState {
            running: false,
            chat: ChatView {
                agent: agent_name.clone(),
                ..Default::default()
            },
            history: Vec::new(),
            scroll: 0,
            follow: true,
            sys_tokens,
            steer_items: Vec::new(),
            queue_items: Vec::new(),
            active_skill: None,
            active_skill_body: None,
            agent_name,
        }
    }

    /// Capture a snapshot of the current live UI variables.
    /// This is the "save" half of the `/task` round-trip.
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot(
        running: bool,
        chat: &ChatView,
        history: &[String],
        scroll: u16,
        follow: bool,
        sys_tokens: u64,
        steer_items: &[String],
        queue_items: &[(i64, String)],
        active_skill: &Option<String>,
        active_skill_body: &Option<String>,
    ) -> Self {
        SessionUiState {
            running,
            chat: chat.clone(),
            history: history.to_vec(),
            scroll,
            follow,
            sys_tokens,
            steer_items: steer_items.to_vec(),
            queue_items: queue_items.to_vec(),
            active_skill: active_skill.clone(),
            active_skill_body: active_skill_body.clone(),
            agent_name: chat.agent.clone(),
        }
    }
}

/// Build a fresh `ChatView` for a resumed session by replaying stored messages
/// as styled markers (user: / say: headers) and reconstructing subagent blocks
/// from persisted `subagent_tasks` records. Used when restoring a session
/// that has no prior UI snapshot.
pub async fn replay_into_chat(
    agent_name: &str,
    messages: &[Message],
    store: &Arc<dyn Store>,
    session_id: &str,
) -> ChatView {
    let mut chat = ChatView {
        agent: agent_name.into(),
        ..Default::default()
    };

    // Load subagent tasks and group by parent_message_id so they can be
    // interleaved after the corresponding assistant message block.
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

    for msg in messages {
        let text: String = msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        if text.is_empty() {
            continue;
        }
        match msg.role {
            Role::User => {
                chat.push_marker(Line::from(Span::styled(
                    "user:",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                let rendered = crate::markdown::render(&text);
                if !rendered.is_empty() {
                    chat.blocks.push(ChatBlock::Marker(rendered));
                }
                chat.push_marker(Line::from(""));
            }
            Role::Assistant => {
                // Use a finalized `Assistant` block so `flatten()` adds the
                // "say:" header + indent and the text is markdown-rendered,
                // matching the live streaming path.
                chat.blocks.push(ChatBlock::Assistant {
                    raw: text.clone(),
                    rendered: crate::markdown::render(&text),
                    done: true,
                });
                // Interleave subagent blocks whose parent_message_id matches
                // this assistant message.
                if let Some(task_list) = tasks_by_parent.remove(&msg.id) {
                    for task in task_list {
                        let block = build_subagent_block(&task, store).await;
                        chat.blocks.push(block);
                    }
                }
            }
            _ => {}
        }
    }

    // Append orphan tasks (no parent_message_id) at the end.
    for task in orphan_tasks {
        let block = build_subagent_block(&task, store).await;
        chat.blocks.push(block);
    }

    chat
}

/// Reconstruct a `ChatBlock::Subagent` from a persisted `SubagentTaskRecord`,
/// including rebuilding the child `ChatView` from stored events.
async fn build_subagent_block(task: &SubagentTaskRecord, store: &Arc<dyn Store>) -> ChatBlock {
    let (done, ok, summary) = match task.status {
        SubagentStatus::Completed => (
            true,
            task.ok.unwrap_or(true),
            task.result.clone().unwrap_or_default(),
        ),
        SubagentStatus::Failed => (true, false, task.result.clone().unwrap_or_default()),
        SubagentStatus::Running => {
            // Interrupted during resume — display as done/failed with a marker.
            (true, false, "(interrupted)".to_string())
        }
    };

    let view = reconstruct_child_view(&task.child_session_id, &task.agent, store).await;

    ChatBlock::Subagent {
        id: task.task_id.clone(),
        child_session_id: task.child_session_id.clone(),
        kind: task.agent.clone(),
        prompt: short(&task.prompt, 90),
        view,
        done,
        ok,
        summary,
    }
}

/// Rebuild a child `ChatView` from persisted events (primary) or messages
/// (fallback) under the child session id.
async fn reconstruct_child_view(
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
            agent: agent_name.into(),
            ..Default::default()
        };
        for rec in &events {
            if let Some(ev) = deserialize_event(&rec.payload) {
                view.apply(&ev);
            }
        }
        return view;
    }

    // Fallback: replay messages.
    let messages = store
        .load_messages(child_session_id)
        .await
        .unwrap_or_default();
    replay_messages(agent_name, &messages)
}

/// Deserialize a `SessionEvent` from a stored event payload.
/// Child events are double-encoded: `Value::String(json_string)`.
fn deserialize_event(payload: &serde_json::Value) -> Option<SessionEvent> {
    match payload {
        serde_json::Value::String(s) => serde_json::from_str::<SessionEvent>(s).ok(),
        other => serde_json::from_value::<SessionEvent>(other.clone()).ok(),
    }
}

/// Text-only message replay (no subagent reconstruction). Used as a fallback
/// for child views without persisted events, and by tests.
fn replay_messages(agent_name: &str, messages: &[Message]) -> ChatView {
    let mut chat = ChatView {
        agent: agent_name.into(),
        ..Default::default()
    };
    for msg in messages {
        let text: String = msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        if text.is_empty() {
            continue;
        }
        match msg.role {
            Role::User => {
                chat.push_marker(Line::from(Span::styled(
                    "user:",
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                let rendered = crate::markdown::render(&text);
                if !rendered.is_empty() {
                    chat.blocks.push(ChatBlock::Marker(rendered));
                }
                chat.push_marker(Line::from(""));
            }
            Role::Assistant => {
                chat.blocks.push(ChatBlock::Assistant {
                    raw: text.clone(),
                    rendered: crate::markdown::render(&text),
                    done: true,
                });
            }
            _ => {}
        }
    }
    chat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chat() -> ChatView {
        let mut c = ChatView {
            agent: "act".into(),
            ..Default::default()
        };
        c.push_marker(ratatui::text::Line::from("hello"));
        c
    }

    #[test]
    fn new_produces_sensible_defaults() {
        let st = SessionUiState::new("plan".into(), 5000);
        assert_eq!(st.agent_name, "plan");
        assert_eq!(st.chat.agent, "plan");
        assert!(!st.running);
        assert!(st.follow);
        assert_eq!(st.scroll, 0);
        assert_eq!(st.sys_tokens, 5000);
        assert!(st.steer_items.is_empty());
        assert!(st.queue_items.is_empty());
        assert!(st.active_skill.is_none());
        assert!(st.history.is_empty());
    }

    #[test]
    fn snapshot_captures_all_fields() {
        let chat = sample_chat();
        let history = vec!["msg1".into(), "msg2".into()];
        let skill = Some("code-review".into());
        let skill_body = Some("review every change carefully".into());
        let steers = vec!["fix bug".into(), "add tests".into(), "refactor".into()];
        let queues = vec![(1_i64, "run lint".into())];

        let snap = SessionUiState::snapshot(
            true,
            &chat,
            &history,
            42,
            false,
            12000,
            &steers,
            &queues,
            &skill,
            &skill_body,
        );

        assert!(snap.running);
        assert_eq!(snap.chat, chat);
        assert_eq!(snap.history, history);
        assert_eq!(snap.scroll, 42);
        assert!(!snap.follow);
        assert_eq!(snap.sys_tokens, 12000);
        assert_eq!(snap.steer_items, steers);
        assert_eq!(snap.queue_items, queues);
        assert_eq!(snap.active_skill, skill);
        assert_eq!(snap.active_skill_body, skill_body);
        assert_eq!(snap.agent_name, "act");
    }

    #[test]
    fn snapshot_is_independent_of_source() {
        // Mutating the source chat after snapshot must not affect the snapshot.
        let mut chat = sample_chat();
        let snap = SessionUiState::snapshot(false, &chat, &[], 0, true, 0, &[], &[], &None, &None);
        chat.push_marker(ratatui::text::Line::from("new line"));
        assert_ne!(snap.chat, chat, "snapshot must be a deep copy");
    }

    #[test]
    fn roundtrip_snapshot_then_compare() {
        // Simulate: snapshot → (logically "store") → compare against fresh values.
        let chat = sample_chat();
        let steers = vec!["s1".into()];
        let queues = vec![(1_i64, "q1".into()), (2_i64, "q2".into())];
        let snap = SessionUiState::snapshot(
            true,
            &chat,
            &["h1".into()],
            10,
            false,
            200,
            &steers,
            &queues,
            &Some("s".into()),
            &Some("body-of-s".into()),
        );
        // After "restore", all fields must match the snapshot.
        assert!(snap.running);
        assert_eq!(snap.chat, chat);
        assert_eq!(snap.history, vec!["h1".to_string()]);
        assert_eq!(snap.scroll, 10);
        assert!(!snap.follow);
        assert_eq!(snap.sys_tokens, 200);
        assert_eq!(snap.steer_items, steers);
        assert_eq!(snap.queue_items, queues);
        assert_eq!(snap.active_skill.as_deref(), Some("s"));
        assert_eq!(snap.active_skill_body.as_deref(), Some("body-of-s"));
    }

    #[test]
    fn replay_renders_plan_handoff_as_markdown() {
        // Simulate the synthetic user message produced by plan_handoff::handoff:
        // the plan markdown is stuffed into a Role::User message.
        let msg = Message::user("u1", "## Plan\n1. do X\n2. do Y");
        let chat = replay_messages("act", &[msg]);
        let lines = chat.flatten();
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        // Headings are rendered as styled text -- the raw "##" markers must
        // not survive into the flattened output.
        assert!(
            !joined.contains("##"),
            "heading must be rendered, not raw; got: {joined}"
        );
        assert!(
            joined.contains("Plan"),
            "plan text must be present; got: {joined}"
        );
    }

    #[test]
    fn replay_renders_assistant_as_markdown_block() {
        let mut msg = Message::assistant("a1");
        msg.blocks
            .push(ContentBlock::text("Here is **bold** text."));
        let chat = replay_messages("act", &[msg]);
        // The replay must produce a finalized Assistant block (markdown-rendered)
        // rather than a plain Marker, so flatten() emits the "say:" header and
        // rendered lines exactly like the live path.
        assert!(
            chat.blocks
                .iter()
                .any(|b| matches!(b, ChatBlock::Assistant { done: true, .. })),
            "assistant replay must produce a finalized Assistant block; got: {:?}",
            chat.blocks
        );
    }
}
