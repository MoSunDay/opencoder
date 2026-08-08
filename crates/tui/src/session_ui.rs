//! Per-session UI state snapshot — saved when switching sessions via `/task`
//! and restored when switching back, so chat history, scroll position, and
//! running status survive a session round-trip.

use crate::chat::ChatView;

mod replay;
pub use replay::replay_into_chat;
pub use replay::replay_messages;

/// Snapshot of all session-specific TUI state. The `input`, `cursor_idx`,
/// `hist_idx`, and `last_esc` are intentionally NOT included — they are
/// interaction-local and reset cleanly on each switch.
#[derive(Clone, PartialEq)]
pub struct SessionUiState {
    pub running: bool,
    pub chat: ChatView,
    pub history: Vec<String>,
    pub scroll: u32,
    pub follow: bool,
    /// Queue/steer panel scroll offset (0 = pinned to top (oldest)).
    pub queue_scroll: u32,
    pub sys_tokens: u64,
    pub queue_items: Vec<(i64, String)>,
    pub active_skill: Option<String>,
    pub active_skill_body: Option<String>,
    pub agent_name: String,
}

impl SessionUiState {
    /// Create a fresh default state for a new session with the given agent.
    pub fn new(agent_name: String, sys_tokens: u64) -> Self {
        let agent_name = crate::terminal_text::sanitize_single_line(&agent_name).into_owned();
        SessionUiState {
            running: false,
            chat: ChatView {
                agent: agent_name.clone(),
                ..Default::default()
            },
            history: Vec::new(),
            scroll: 0,
            follow: true,
            queue_scroll: 0,
            sys_tokens,
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
        scroll: u32,
        follow: bool,
        queue_scroll: u32,
        sys_tokens: u64,
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
            queue_scroll,
            sys_tokens,
            queue_items: queue_items.to_vec(),
            active_skill: active_skill.clone(),
            active_skill_body: active_skill_body.clone(),
            agent_name: chat.agent.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::replay::replay_messages;
    use super::*;
    use crate::chat::ChatBlock;
    use opencoder_core::{ContentBlock, Message, Role};

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
        assert!(st.chat.steer_items.is_empty());
        assert!(st.queue_items.is_empty());
        assert!(st.active_skill.is_none());
        assert!(st.history.is_empty());
    }

    #[test]
    fn snapshot_captures_all_fields() {
        let mut chat = sample_chat();
        let history = vec!["msg1".into(), "msg2".into()];
        let skill = Some("code-review".into());
        let skill_body = Some("review every change carefully".into());
        let steers = vec![
            (10_i64, "fix bug".into()),
            (11, "add tests".into()),
            (12, "refactor".into()),
        ];
        let queues = vec![(1_i64, "run lint".into())];
        chat.steer_items = steers.clone();

        let snap = SessionUiState::snapshot(
            true,
            &chat,
            &history,
            42,
            false,
            7,
            12000,
            &queues,
            &skill,
            &skill_body,
        );

        assert!(snap.running);
        assert_eq!(snap.chat, chat);
        assert_eq!(snap.history, history);
        assert_eq!(snap.scroll, 42);
        assert!(!snap.follow);
        assert_eq!(snap.queue_scroll, 7);
        assert_eq!(snap.sys_tokens, 12000);
        assert_eq!(snap.chat.steer_items, steers);
        assert_eq!(snap.queue_items, queues);
        assert_eq!(snap.active_skill, skill);
        assert_eq!(snap.active_skill_body, skill_body);
        assert_eq!(snap.agent_name, "act");
    }

    fn make_user(id: &str, text: &str, synthetic: bool) -> Message {
        let mut m = Message::user(id, text);
        m.synthetic = synthetic;
        m
    }

    #[test]
    fn replay_skips_synthetic_user_messages() {
        // Synthetic user messages (steer/queue promotion, plan->act handoff, compaction
        // summary) must NOT appear as visible `user:` blocks on resume/replay.
        let msgs = vec![
            make_user("u1", "real prompt", false),
            make_user("u2", "[synthetic steer body]", true),
            make_user("u3", "another real prompt", false),
        ];
        let chat = replay_messages("act", &msgs);
        let flat = chat.flatten();
        let joined: String = flat
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect::<String>();
        assert!(joined.contains("real prompt"));
        assert!(joined.contains("another real prompt"));
        assert!(
            !joined.contains("synthetic steer body"),
            "synthetic user message leaked into replay: {joined}"
        );
    }

    #[test]
    fn snapshot_is_independent_of_source() {
        // Mutating the source chat after snapshot must not affect the snapshot.
        let mut chat = sample_chat();
        let snap = SessionUiState::snapshot(false, &chat, &[], 0, true, 0, 0, &[], &None, &None);
        chat.push_marker(ratatui::text::Line::from("new line"));
        assert_ne!(snap.chat, chat, "snapshot must be a deep copy");
    }

    #[test]
    fn roundtrip_snapshot_then_compare() {
        // Simulate: snapshot → (logically "store") → compare against fresh values.
        let mut chat = sample_chat();
        let steers = vec![(7_i64, "s1".into())];
        let queues = vec![(1_i64, "q1".into()), (2_i64, "q2".into())];
        chat.steer_items = steers.clone();
        let snap = SessionUiState::snapshot(
            true,
            &chat,
            &["h1".into()],
            10,
            false,
            4,
            200,
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
        assert_eq!(snap.queue_scroll, 4);
        assert_eq!(snap.sys_tokens, 200);
        assert_eq!(snap.chat.steer_items, steers);
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

    #[test]
    fn replay_reconstructs_tool_blocks() {
        // Assistant message with a ToolUse, followed by a Role::Tool message
        // carrying the matching ToolResult. Replay must produce a
        // ChatBlock::Tool with the correct id, header, and appended output.
        let mut asst = Message::assistant("a1");
        asst.blocks.push(ContentBlock::text("Running a command."));
        asst.blocks.push(ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "echo hi"}),
        });
        let mut tool_msg = Message::assistant("tool1");
        tool_msg.role = Role::Tool;
        tool_msg.blocks = vec![ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            content: "hi".into(),
            is_error: false,
            images: Vec::new(),
        }];
        let chat = replay_messages("act", &[asst, tool_msg]);
        let tools: Vec<_> = chat
            .blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::Tool {
                    id, header, output, ..
                } => Some((id, header, output)),
                _ => None,
            })
            .collect();
        assert_eq!(tools.len(), 1, "expected one tool block");
        assert_eq!(tools[0].0, "t1");
        let text: String = tools[0]
            .1
            .spans
            .iter()
            .chain(tools[0].2.iter().flat_map(|l| l.spans.iter()))
            .map(|s| s.content.clone())
            .collect();
        assert!(
            text.contains("echo hi"),
            "header should show command: {text}"
        );
        assert!(text.contains("hi"), "output should be appended: {text}");
    }

    #[test]
    fn replay_tool_only_assistant_not_skipped() {
        // An assistant turn with only a ToolUse (no Text) must not be skipped
        // — previously the `text.is_empty() { continue }` guard dropped it.
        let mut asst = Message::assistant("a1");
        asst.blocks.push(ContentBlock::ToolUse {
            id: "t9".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        });
        let chat = replay_messages("act", &[asst]);
        assert!(
            chat.blocks
                .iter()
                .any(|b| matches!(b, ChatBlock::Tool { id, .. } if id == "t9")),
            "tool-only assistant turn must not be skipped; got: {:?}",
            chat.blocks
        );
    }

    #[test]
    fn replay_parallel_tools_paired_by_id() {
        // Two tool calls in one assistant message; results arrive in a
        // separate Role::Tool message in reverse order. Each result must land
        // in its own block, paired by tool_use_id.
        let mut asst = Message::assistant("a1");
        asst.blocks.push(ContentBlock::ToolUse {
            id: "p1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "echo one"}),
        });
        asst.blocks.push(ContentBlock::ToolUse {
            id: "p2".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "echo two"}),
        });
        let mut tool_msg = Message::assistant("t1");
        tool_msg.role = Role::Tool;
        tool_msg.blocks = vec![
            ContentBlock::ToolResult {
                tool_use_id: "p2".into(),
                content: "two".into(),
                is_error: false,
                images: Vec::new(),
            },
            ContentBlock::ToolResult {
                tool_use_id: "p1".into(),
                content: "one".into(),
                is_error: false,
                images: Vec::new(),
            },
        ];
        let chat = replay_messages("act", &[asst, tool_msg]);
        let tools: Vec<_> = chat
            .blocks
            .iter()
            .filter_map(|b| match b {
                ChatBlock::Tool { id, output, .. } => Some((id, output)),
                _ => None,
            })
            .collect();
        assert_eq!(tools.len(), 2, "expected two tool blocks");
        assert_eq!(tools[0].0, "p1");
        assert_eq!(tools[1].0, "p2");
        let out0: String = tools[0]
            .1
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        let out1: String = tools[1]
            .1
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(out0.contains("one"), "p1 output: {out0}");
        assert!(out1.contains("two"), "p2 output: {out1}");
    }

    // -----------------------------------------------------------------------
    // P0: Tool-returned images render inline on the replay path.
    // When a persisted Tool message carries images, replay must produce
    // ChatBlock::Image blocks alongside the tool output.
    // -----------------------------------------------------------------------

    fn tiny_png_data_uri() -> String {
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==".into()
    }

    #[test]
    fn replay_tool_message_with_images_renders_image_block() {
        use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
        let uri = tiny_png_data_uri();
        let tool_msg = Message {
            id: "m-tool".into(),
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "Loaded image: cat.png (0.1 KiB)".into(),
                is_error: false,
                images: vec![uri],
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        };
        let chat = replay_messages("act", &[tool_msg]);
        let images: Vec<_> = chat
            .blocks
            .iter()
            .filter(|b| matches!(b, ChatBlock::Image { .. }))
            .collect();
        assert_eq!(
            images.len(),
            1,
            "replayed tool message with one image must produce one Image block"
        );
    }

    #[test]
    fn replay_tool_message_without_images_no_image_block() {
        use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
        let tool_msg = Message {
            id: "m-tool2".into(),
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "t2".into(),
                content: "command output".into(),
                is_error: false,
                images: Vec::new(),
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        };
        let chat = replay_messages("act", &[tool_msg]);
        let images: Vec<_> = chat
            .blocks
            .iter()
            .filter(|b| matches!(b, ChatBlock::Image { .. }))
            .collect();
        assert!(
            images.is_empty(),
            "tool message without images must not produce Image blocks"
        );
    }
}

#[cfg(test)]
mod subagent_block_tests;

#[cfg(test)]
mod image_prefetch_tests;

#[cfg(test)]
mod replay_duration_tests;

#[cfg(test)]
#[path = "session_ui/terminal_safety_tests.rs"]
mod terminal_safety_tests;
