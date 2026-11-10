//! Wire-level contract tests for tool-call pairing in `ChatRequest::to_body`.
//!
//! `lower_messages` plus the sanitize pass must guarantee the OpenAI pairing
//! contract on the emitted body: every assistant `tool_calls` id is answered
//! by exactly one immediately-following `role:"tool"` message, and no `tool`
//! message exists without its call. Corrupted transcripts (hard-cancel /
//! compaction boundary artifacts) are repaired by dropping/stripping in place
//! — never by reordering, which would rewrite the conversation's history.

use std::collections::HashSet;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::{lower_messages, ChatRequest, RequestPurpose};
use serde_json::{json, Value};

// --- helpers ---

fn req(messages: Vec<Message>) -> ChatRequest {
    ChatRequest {
        purpose: RequestPurpose::Conversation,
        model: "m".to_string(),
        messages,
        tools: Vec::new(),
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

fn base_msg(id: &str, role: Role, blocks: Vec<ContentBlock>) -> Message {
    Message {
        provider_state: None,
        display: None,
        id: id.into(),
        role,
        blocks,
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    }
}

/// Assistant message with an optional text block plus one `ToolUse` block per
/// `(id, name)` pair.
fn assistant_calls(id: &str, text: Option<&str>, calls: &[(&str, &str)]) -> Message {
    let mut blocks: Vec<ContentBlock> = text
        .map(|t| ContentBlock::Text { text: t.into() })
        .into_iter()
        .collect();
    blocks.extend(calls.iter().map(|&(call_id, name)| ContentBlock::ToolUse {
        id: call_id.into(),
        name: name.into(),
        input: json!({}),
    }));
    base_msg(id, Role::Assistant, blocks)
}

fn tool_result_block(id: &str, content: &str, images: Vec<String>) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: id.into(),
        content: content.into(),
        is_error: false,
        images,
    }
}

fn tool_result(id: &str, content: &str) -> Message {
    base_msg(
        "mt",
        Role::Tool,
        vec![tool_result_block(id, content, Vec::new())],
    )
}

/// `Role::User` message embedding a tool result (optionally with returned
/// images) plus a text block; `lower_messages` emits the `tool` message first,
/// then an image user turn (when images exist), then the text turn.
fn user_embedded_result_and_text(
    id: &str,
    tool_id: &str,
    content: &str,
    images: Vec<String>,
    text: &str,
) -> Message {
    base_msg(
        id,
        Role::User,
        vec![
            tool_result_block(tool_id, content, images),
            ContentBlock::Text { text: text.into() },
        ],
    )
}

fn user_text(id: &str, text: &str) -> Message {
    base_msg(id, Role::User, vec![ContentBlock::text(text)])
}

fn body_messages(body: &Value) -> &Vec<Value> {
    body.get("messages")
        .and_then(Value::as_array)
        .expect("request body must carry a messages array")
}

/// Walk the wire stream keeping the set of currently-open call ids: an
/// assistant resets it to its `tool_calls` ids, a `tool` message must answer
/// one of them (consuming it), any other role requires the set to be empty,
/// and the stream must not end with unanswered calls.
fn assert_pairing_well_formed(msgs: &[Value]) {
    let mut open: HashSet<String> = HashSet::new();
    for (i, msg) in msgs.iter().enumerate() {
        match msg.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                open = msg
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| {
                        calls
                            .iter()
                            .filter_map(|c| c.get("id").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Some("tool") => {
                let id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| {
                        panic!("message[{i}] has role tool but no string tool_call_id")
                    });
                assert!(
                    open.remove(id),
                    "message[{i}] answers unknown/already-answered call {id:?}; open: {open:?}"
                );
            }
            role => {
                assert!(
                    open.is_empty(),
                    "message[{i}] role {role:?} interrupts unanswered calls {open:?}"
                );
            }
        }
    }
    assert!(
        open.is_empty(),
        "stream ends with unanswered calls {open:?}"
    );
}

fn tool_messages_with_id(msgs: &[Value], id: &str) -> usize {
    msgs.iter()
        .filter(|m| m["role"] == "tool" && m["tool_call_id"] == id)
        .count()
}

fn assistant_tool_call_ids(msgs: &[Value]) -> Vec<String> {
    msgs.iter()
        .filter(|m| m["role"] == "assistant")
        .filter_map(|m| m.get("tool_calls"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|c| c.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

// --- tests ---

#[test]
fn legal_transcript_passes_through_unchanged() {
    let messages = vec![
        base_msg("m0", Role::System, vec![ContentBlock::text("sys")]),
        user_text("m1", "hi"),
        assistant_calls("m2", None, &[("a", "get"), ("b", "put")]),
        base_msg(
            "m3",
            Role::Tool,
            vec![
                tool_result_block("a", "ra", Vec::new()),
                tool_result_block("b", "rb", Vec::new()),
            ],
        ),
        assistant_calls("m4", Some("done"), &[]),
        user_text("m5", "again"),
    ];
    let body = req(messages.clone()).to_body();
    let msgs = body_messages(&body);
    assert_eq!(
        Value::Array(msgs.clone()),
        json!(lower_messages(&messages)),
        "legal transcript must reach the body byte-identical to plain lowering"
    );
    assert_pairing_well_formed(msgs);
}

#[test]
fn orphan_tool_at_head_is_dropped() {
    // A tool result ahead of any assistant turn is a compaction-boundary
    // artifact; strict providers reject it, so it must vanish.
    let messages = vec![
        tool_result("x", "orphan"),
        user_text("m1", "hi"),
        assistant_calls("m2", None, &[("a", "get")]),
        tool_result("a", "ra"),
        user_text("m3", "again"),
    ];
    let body = req(messages).to_body();
    let msgs = body_messages(&body);
    assert_ne!(
        msgs[0]["role"], "tool",
        "no tool message may lead the stream"
    );
    assert_eq!(tool_messages_with_id(msgs, "x"), 0, "orphan must be gone");
    assert_eq!(
        tool_messages_with_id(msgs, "a"),
        1,
        "the legal exchange behind it stays intact"
    );
    assert_pairing_well_formed(msgs);
}

#[test]
fn user_between_calls_and_results_strips_unanswered_call() {
    // The user turn closes the answer window: call a is answered before it,
    // call b's result arrives after it and must be dropped, and b is stripped
    // from the assistant so the body stays pairing-legal.
    let messages = vec![
        assistant_calls("m1", None, &[("a", "f"), ("b", "g")]),
        user_embedded_result_and_text("m2", "a", "ra", Vec::new(), "partial answer"),
        tool_result("b", "rb"),
    ];
    let body = req(messages).to_body();
    let msgs = body_messages(&body);
    assert_eq!(
        tool_messages_with_id(msgs, "a"),
        1,
        "answered call keeps its tool message"
    );
    assert_eq!(
        tool_messages_with_id(msgs, "b"),
        0,
        "result after the user turn is dropped"
    );
    assert_eq!(
        assistant_tool_call_ids(msgs),
        vec!["a".to_string()],
        "unanswered call must be stripped from the assistant"
    );
    assert_pairing_well_formed(msgs);
}

#[test]
fn image_user_message_between_batches_is_preserved() {
    // Same repair as above, but the answered result carries an image: the
    // rehomed image user turn sits between the batches and must survive the
    // repair exactly once.
    let messages = vec![
        assistant_calls("m1", None, &[("a", "f"), ("b", "g")]),
        user_embedded_result_and_text(
            "m2",
            "a",
            "ra",
            vec!["data:image/png;base64,AAA".to_string()],
            "screenshot attached",
        ),
        tool_result("b", "rb"),
    ];
    let body = req(messages).to_body();
    let msgs = body_messages(&body);
    assert_eq!(tool_messages_with_id(msgs, "a"), 1);
    assert_eq!(tool_messages_with_id(msgs, "b"), 0);
    assert_eq!(assistant_tool_call_ids(msgs), vec!["a".to_string()]);
    let image_turns = msgs
        .iter()
        .filter(|m| {
            m["role"] == "user"
                && m.get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| parts.iter().any(|p| p["type"] == "image_url"))
        })
        .count();
    assert_eq!(
        image_turns, 1,
        "the rehomed image user turn must survive exactly once"
    );
    assert_pairing_well_formed(msgs);
}

#[test]
fn unanswered_call_on_text_assistant_stripped_message_kept() {
    // An assistant with its own text survives the strip: only the unanswered
    // `tool_calls` key goes, and no empty (spec-invalid) array is left.
    let messages = vec![
        assistant_calls("m1", Some("I will call a tool"), &[("a", "f")]),
        user_text("m2", "go on"),
    ];
    let body = req(messages).to_body();
    let msgs = body_messages(&body);
    let assistant = msgs
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("text-bearing assistant must survive the strip");
    assert_eq!(assistant["content"], "I will call a tool");
    assert!(
        assistant.get("tool_calls").is_none(),
        "stripped assistant must lose the tool_calls key entirely"
    );
    assert_eq!(
        assistant_tool_call_ids(msgs),
        Vec::<String>::new(),
        "the unanswered call is reflected by its absence"
    );
    assert_pairing_well_formed(msgs);
}

#[test]
fn tool_call_only_assistant_without_results_removed() {
    // A tool-call-only assistant (content null) with no answers carries no
    // payload worth keeping: the whole message is dropped.
    let messages = vec![
        assistant_calls("m1", None, &[("a", "f")]),
        user_text("m2", "next"),
    ];
    let body = req(messages).to_body();
    let msgs = body_messages(&body);
    assert_eq!(
        assistant_tool_call_ids(msgs),
        Vec::<String>::new(),
        "no tool_call id may remain unanswered on the wire"
    );
    assert_eq!(msgs.len(), 1, "only the user message remains");
    assert_eq!(msgs[0]["role"], "user");
    assert_pairing_well_formed(msgs);
}

#[test]
fn duplicate_result_kept_once_and_early_result_dropped() {
    // The result for b arrives before the call exists (dropped), and a's
    // result is replayed twice (only the first counts).
    let messages = vec![
        tool_result("b", "early"),
        assistant_calls("m1", None, &[("a", "f"), ("b", "g")]),
        tool_result("a", "ra"),
        tool_result("a", "ra again"),
    ];
    let body = req(messages).to_body();
    let msgs = body_messages(&body);
    assert_eq!(
        tool_messages_with_id(msgs, "a"),
        1,
        "duplicate answers keep only the first"
    );
    assert_eq!(
        tool_messages_with_id(msgs, "b"),
        0,
        "answer arriving before its call is dropped"
    );
    assert_eq!(assistant_tool_call_ids(msgs), vec!["a".to_string()]);
    assert_pairing_well_formed(msgs);
}
