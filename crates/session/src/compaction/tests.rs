use super::*;
use opencoder_core::{ContentBlock, MessageUsage};

fn tool_msg(id: &str, tool_use_id: &str) -> Message {
    Message {
        id: id.into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: "x".into(),
            is_error: false,
            images: Vec::new(),
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

fn assistant_with_tool(id: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::ToolUse {
        id: "tc".into(),
        name: "bash".into(),
        input: serde_json::json!({}),
    });
    m
}

#[test]
fn split_index_assistant_after_tool_is_turn_boundary() {
    // Single user task with 3 tool roundtrips — common coding-agent shape.
    // With the old code this would return 0 (only 1 real user message).
    let msgs = vec![
        Message::user("u1", "task"),
        assistant_with_tool("a1"),
        tool_msg("t1", "tc"),
        assistant_with_tool("a2"),
        tool_msg("t2", "tc"),
        assistant_with_tool("a3"),
        tool_msg("t3", "tc"),
        Message::assistant("a4"),
    ];
    // turn_starts = [0, 3, 5, 7], tail=2 → split = turn_starts[2] = 5
    let split = split_index(&msgs, 2);
    assert!(
        split > 0,
        "tool-intensive single-user session must be splittable, got split={split}"
    );
    assert_eq!(split, 5);
}

#[test]
fn split_index_multi_user_unchanged() {
    // Classic multi-user session — split point must not change.
    let msgs = vec![
        Message::user("u1", "first task"),
        Message::assistant("a1"),
        Message::user("u2", "second task"),
        Message::assistant("a2"),
        Message::user("u3", "third task"),
        Message::assistant("a3"),
    ];
    // turn_starts = [0, 2, 4] (all real user messages)
    // tail=2 → split = turn_starts[1] = 2
    assert_eq!(split_index(&msgs, 2), 2);
    // tail=1 → split = turn_starts[2] = 4
    assert_eq!(split_index(&msgs, 1), 4);
}

#[test]
fn split_index_returns_zero_when_too_few_turns() {
    // Single user + one tool roundtrip → turn_starts=[0, 3], tail=2 → 0.
    let msgs = vec![
        Message::user("u1", "task"),
        assistant_with_tool("a1"),
        tool_msg("t1", "tc"),
        Message::assistant("a2"),
    ];
    assert_eq!(split_index(&msgs, 2), 0);
}

#[test]
fn split_index_mixed_user_and_tool_turns() {
    // A session with both real user turns and tool roundtrips.
    let msgs = vec![
        Message::user("u1", "task1"),
        assistant_with_tool("a1"),
        tool_msg("t1", "tc"),
        assistant_with_tool("a2"),
        tool_msg("t2", "tc"),
        Message::user("u2", "task2"),
        assistant_with_tool("a3"),
        tool_msg("t3", "tc"),
        Message::assistant("a4"),
    ];
    // turn_starts = [0, 3, 5, 8], tail=2 → split = turn_starts[2] = 5
    assert_eq!(split_index(&msgs, 2), 5);
    // tail=1 → split = turn_starts[3] = 8
    assert_eq!(split_index(&msgs, 1), 8);
}

#[test]
fn compaction_split_fallback_summarizes_oldest_turn() {
    // Two turns, tail_turns=2: ideal split_index returns 0 (too few
    // turns), but the over-budget fallback must still split — summarizing
    // the first turn and keeping the second.
    // turn_starts = [0, 2], fallback -> turn_starts[1] = 2.
    let msgs = vec![
        Message::user("u1", "first"),
        Message::assistant("a1"),
        Message::user("u2", "second"),
        Message::assistant("a2"),
    ];
    assert_eq!(compaction_split(&msgs, 2), Some(2));
    // head = msgs[..2] (first turn), tail = msgs[2..] (second turn).
}

#[test]
fn compaction_split_fallback_two_tool_turns() {
    // turn_starts = [0, 3], tail_turns=2 -> ideal returns 0; fallback
    // -> turn_starts[1] = 3 (keep the second turn, summarize the first).
    let msgs = vec![
        Message::user("u1", "task"),
        assistant_with_tool("a1"),
        tool_msg("t1", "tc"),
        Message::user("u2", "more"),
        Message::assistant("a2"),
    ];
    assert_eq!(compaction_split(&msgs, 2), Some(3));
}

#[test]
fn compaction_split_single_turn_keeps_last_message() {
    // One turn (turn_starts=[0]), two messages: summarize the first
    // message, keep the most recent one as the tail.
    let msgs = vec![Message::user("u1", "big paste"), Message::assistant("a1")];
    assert_eq!(compaction_split(&msgs, 2), Some(1));
}

#[test]
fn compaction_split_single_message_is_no_op() {
    // A lone message cannot be summarized without destroying the only
    // context — this is the one genuine no-op.
    let msgs = vec![Message::user("u1", "big paste")];
    assert_eq!(compaction_split(&msgs, 2), None);
    assert_eq!(compaction_split(&[], 2), None);
}

#[test]
fn compaction_split_matches_ideal_when_enough_turns() {
    // Three turns, tail_turns=2 -> ideal path equals split_index.
    let msgs = vec![
        Message::user("u1", "a"),
        Message::assistant("a1"),
        Message::user("u2", "b"),
        Message::assistant("a2"),
        Message::user("u3", "c"),
        Message::assistant("a3"),
    ];
    // turn_starts = [0, 2, 4]; tail=2 -> turn_starts[1] = 2
    assert_eq!(compaction_split(&msgs, 2), Some(2));
    assert_eq!(compaction_split(&msgs, 2).unwrap(), split_index(&msgs, 2));
}

/// Issue #3 (root cause A): the compaction-summary LLM stream must honor
/// the session cancel token. A double-Esc / web interrupt mid-compaction
/// must abort promptly and leave the transcript untouched (compaction only
/// rewrites `messages` after the summary returns Ok).
#[tokio::test]
async fn compact_honors_cancel_and_leaves_messages_intact() {
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
    use tokio_util::sync::CancellationToken;

    let cancel = CancellationToken::new();
    cancel.cancel();
    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("partial ".into()),
        LlmEvent::TextDelta("summary".into()),
        LlmEvent::Completed {
            text: "partial summary".into(),
            tool_calls: Vec::<CompletedToolCall>::new(),
            usage: Some(Usage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                ..Default::default()
            }),
        },
    ]));
    let agent = resolve_agent("act").expect("act agent");
    let mut s = SessionState::new(
        "compact-cancel",
        agent,
        Config {
            model: "main/glm-5.2".into(),
            ..Config::default()
        },
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel);
    // Two turns so `compaction_split` returns a real head/tail split.
    s.messages.push(Message::user("u1", "first turn"));
    s.messages.push(Message::assistant("a1"));
    s.messages.push(Message::user("u2", "second turn"));
    s.messages.push(Message::assistant("a2"));
    let before = s.messages.len();

    let mut events: Vec<SessionEvent> = Vec::new();
    let outcome = compact(&mut s, &HashMap::new(), &mut |ev| events.push(ev)).await;

    assert!(outcome.is_err(), "compaction must abort when cancelled");
    assert_eq!(
        s.messages.len(),
        before,
        "transcript must be untouched when compaction is cancelled"
    );
    // No synthetic compaction-summary message was prepended.
    assert!(s
        .messages
        .iter()
        .all(|m| { !(m.synthetic && m.text().starts_with("[Conversation summary so far]")) }));
    // The cancel arm emits an interrupted status before bailing.
    assert!(events
        .iter()
        .any(|ev| matches!(ev, SessionEvent::Status(msg) if msg == "interrupted")));
}
