//! Per-session UI state snapshot — saved when switching sessions via `/task`
//! and restored when switching back, so chat history, scroll position, and
//! running status survive a session round-trip.

use opencode_core::{ContentBlock, Message, Role};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::chat::ChatView;

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
    pub context_used: u64,
    pub sys_tokens: u64,
    pub steer_items: Vec<String>,
    pub queue_items: Vec<String>,
    pub active_skill: Option<String>,
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
            context_used: 0,
            sys_tokens,
            steer_items: Vec::new(),
            queue_items: Vec::new(),
            active_skill: None,
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
        context_used: u64,
        sys_tokens: u64,
        steer_items: &[String],
        queue_items: &[String],
        active_skill: &Option<String>,
    ) -> Self {
        SessionUiState {
            running,
            chat: chat.clone(),
            history: history.to_vec(),
            scroll,
            follow,
            context_used,
            sys_tokens,
            steer_items: steer_items.to_vec(),
            queue_items: queue_items.to_vec(),
            active_skill: active_skill.clone(),
            agent_name: chat.agent.clone(),
        }
    }
}

/// Build a fresh `ChatView` for a resumed session by replaying stored messages
/// as styled markers (user: / say: headers). Used when restoring a session
/// that has no prior UI snapshot.
pub fn replay_into_chat(agent_name: &str, messages: &[Message]) -> ChatView {
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
                    format!("user: {text}"),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                chat.push_marker(Line::from(""));
            }
            Role::Assistant => {
                chat.push_marker(Line::from(Span::styled(
                    "say:",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                )));
                chat.push_marker(Line::from(format!("    {text}")));
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
        assert_eq!(st.context_used, 0);
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
        let steers = vec!["fix bug".into(), "add tests".into(), "refactor".into()];
        let queues = vec!["run lint".into()];

        let snap = SessionUiState::snapshot(
            true,
            &chat,
            &history,
            42,
            false,
            8000,
            12000,
            &steers,
            &queues,
            &skill,
        );

        assert!(snap.running);
        assert_eq!(snap.chat, chat);
        assert_eq!(snap.history, history);
        assert_eq!(snap.scroll, 42);
        assert!(!snap.follow);
        assert_eq!(snap.context_used, 8000);
        assert_eq!(snap.sys_tokens, 12000);
        assert_eq!(snap.steer_items, steers);
        assert_eq!(snap.queue_items, queues);
        assert_eq!(snap.active_skill, skill);
        assert_eq!(snap.agent_name, "act");
    }

    #[test]
    fn snapshot_is_independent_of_source() {
        // Mutating the source chat after snapshot must not affect the snapshot.
        let mut chat = sample_chat();
        let snap = SessionUiState::snapshot(false, &chat, &[], 0, true, 0, 0, &[], &[], &None);
        chat.push_marker(ratatui::text::Line::from("new line"));
        assert_ne!(snap.chat, chat, "snapshot must be a deep copy");
    }

    #[test]
    fn roundtrip_snapshot_then_compare() {
        // Simulate: snapshot → (logically "store") → compare against fresh values.
        let chat = sample_chat();
        let steers = vec!["s1".into()];
        let queues = vec!["q1".into(), "q2".into()];
        let snap = SessionUiState::snapshot(
            true, &chat, &["h1".into()], 10, false, 100, 200, &steers, &queues, &Some("s".into()),
        );
        // After "restore", all fields must match the snapshot.
        assert!(snap.running);
        assert_eq!(snap.chat, chat);
        assert_eq!(snap.history, vec!["h1".to_string()]);
        assert_eq!(snap.scroll, 10);
        assert!(!snap.follow);
        assert_eq!(snap.context_used, 100);
        assert_eq!(snap.sys_tokens, 200);
        assert_eq!(snap.steer_items, steers);
        assert_eq!(snap.queue_items, queues);
        assert_eq!(snap.active_skill.as_deref(), Some("s"));
    }
}
