//! P1 functional tests for compaction + model selection behavior.
//! All driven by MockChatClient — zero network, fully deterministic.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, MessageUsage, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{compaction::should_compact, run, SessionState};

fn client_with_default_done(text: &str) -> Arc<MockChatClient> {
    Arc::new(MockChatClient::new().with_default(vec![done_event(text)]))
}

fn done_event(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        }),
    }
}

fn base_config() -> Config {
    Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    }
}

async fn session_with(
    config: Config,
    client: Arc<dyn ChatStream>,
) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new(
        "test-session",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );
    (dir, s)
}

fn big_user_message(id: &str, chars: usize) -> Message {
    let text: String = "a".repeat(chars);
    Message::user(id, text)
}

/// An assistant message carrying a tool-use block (simulates a tool-call turn).
fn assistant_with_tool(id: &str, tool_id: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::ToolUse {
        id: tool_id.into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo hi"}),
    });
    m
}

/// A tool-result message (simulates the tool execution result).
fn tool_result(id: &str, tool_id: &str, content: &str) -> Message {
    Message {
        id: id.into(),
        role: Role::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: tool_id.into(),
            content: content.into(),
            is_error: false,
        }],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    }
}

#[tokio::test]
async fn compaction_triggers_by_token_estimate_without_any_reported_usage() {
    // Small window so a single large message trips the estimate-based trigger
    // on round 1, even though last_usage is zero (no provider call yet).
    let mut config = base_config();
    config.context_limit = Some(2_000);
    config.compaction.reserved = 200;
    config.compaction.context_threshold = 10_000; // larger than usable, so usable binds
    let (_dir, mut s) = session_with(config, client_with_default_done("ok")).await;
    // 8000 chars ≈ 2000 tokens + overhead → exceeds usable(1800)
    s.messages.push(big_user_message("u1", 8_000));

    assert!(
        s.last_usage.total_tokens == 0,
        "precondition: no usage reported yet"
    );
    assert!(should_compact(&s), "estimate alone must trigger compaction");
}

#[tokio::test]
async fn reserved_budget_actually_shrinks_usable_window() {
    fn mk(reserved: u64) -> bool {
        let mut config = base_config();
        config.context_limit = Some(2_000);
        config.compaction.context_threshold = 10_000; // usable binds
        config.compaction.reserved = reserved;
        // build synchronously: should_compact is sync
        let dir = tempfile::tempdir().unwrap();
        let agent = resolve_agent("act").unwrap();
        let client: Arc<dyn ChatStream> = client_with_default_done("ok");
        let mut s = SessionState::new("x", agent, config, client, dir.path().to_path_buf());
        // ~1400 estimated tokens
        s.messages.push(big_user_message("u1", 5_500));
        should_compact(&s)
    }
    // reserved=200 → usable=1800 → 1400 < 1800 → no compact
    assert!(!mk(200), "with small reserved, 1400 tokens should NOT trip");
    // reserved=1100 → usable=900 → 1400 >= 900 → compact
    assert!(mk(1_100), "with large reserved, 1400 tokens MUST trip");
}

#[tokio::test]
async fn small_model_is_used_for_compaction_summary_call() {
    let mut config = base_config();
    config.small_model = Some("cheap/mini".into());
    // Force compaction to fire & have room to summarize.
    config.context_limit = Some(2_000);
    config.compaction.reserved = 100;
    config.compaction.context_threshold = 10_000;
    config.compaction.tail_turns = 1; // need >tail user msgs to actually split

    let mock = Arc::new(
        MockChatClient::new()
            // first call = main turn (returns a tool-less done so the loop exits after),
            // but compaction runs BEFORE the turn. Provide a summary script first.
            .push_script(vec![LlmEvent::Completed {
                text: "SUMMARY".into(),
                tool_calls: Vec::<CompletedToolCall>::new(),
                usage: None,
            }])
            .push_script(vec![done_event("done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(config, client).await;
    s.messages.push(big_user_message("u1", 8_000));
    s.messages.push(Message::user("u2", "keep me tail"));

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(!reqs.is_empty(), "at least the compaction call must happen");
    assert_eq!(
        reqs[0].model, "mini",
        "compaction summarize must use small_model id, got {}",
        reqs[0].model
    );
}

#[tokio::test]
async fn model_switch_takes_effect_on_next_request_body() {
    let mock = Arc::new(MockChatClient::new().with_default(vec![done_event("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(base_config(), client).await;
    assert_eq!(s.model, "glm-5.2");

    // first turn with the initial model
    run(&mut s, "first".into(), |_| {}).await.unwrap();

    // switch the session model mid-session
    s.model = "switched/claude".into();

    // next turn must carry the new model in the request body
    run(&mut s, "second".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(reqs.len() >= 2, "need >=2 calls");
    let last = reqs.last().unwrap();
    assert_eq!(
        last.model, "switched/claude",
        "switched model must reach the provider"
    );
}

#[tokio::test]
async fn compaction_disabled_does_not_trigger() {
    let mut config = base_config();
    config.compaction.auto = false;
    config.context_limit = Some(100);
    config.compaction.reserved = 0;
    let (_dir, mut s) = session_with(config, client_with_default_done("ok")).await;
    s.messages.push(big_user_message("u1", 100_000));
    assert!(
        !should_compact(&s),
        "auto=false disables compaction entirely"
    );
}

#[tokio::test]
async fn reported_tokens_uses_input_only_not_total() {
    let mut config = base_config();
    config.context_limit = Some(10_000);
    config.compaction.context_threshold = 10_000; // larger than usable → usable binds
    config.compaction.reserved = 2_000; // usable = 8_000

    let (_dir, mut s) = session_with(config, client_with_default_done("ok")).await;
    // Small messages so the estimate-based trigger does NOT fire.
    s.messages.push(Message::user("u1", "small"));

    // Simulate a turn where input is well under budget but output is huge.
    // total_tokens would exceed budget, but input_tokens alone does not.
    s.last_usage = Usage {
        input_tokens: 3_000,
        output_tokens: 9_000,
        total_tokens: 12_000,
    };

    assert!(
        !should_compact(&s),
        "must NOT trigger on output-heavy turns: input=3k < budget=8k, even though total=12k > 8k"
    );

    // Now set input_tokens above budget — should trigger.
    s.last_usage = Usage {
        input_tokens: 9_000,
        output_tokens: 0,
        total_tokens: 9_000,
    };
    assert!(
        should_compact(&s),
        "must trigger when input_tokens exceeds budget"
    );
}

#[tokio::test]
async fn compaction_fires_in_tool_intensive_single_user_session() {
    // Regression: with the old split_index (only real user messages counted as
    // turn boundaries), a single-user session with many tool roundtrips could
    // never compact — the big transcript kept growing until the provider
    // rejected it for context length. With the fix, assistant-after-tool is a
    // turn boundary, so compaction fires and the head (incl. the original task)
    // gets summarized.
    let mut config = base_config();
    config.context_limit = Some(2_000);
    config.compaction.reserved = 100; // usable = 1900
    config.compaction.context_threshold = 10_000; // usable binds
    config.compaction.tail_turns = 2;

    // Mock: call 1 = compaction summary, call 2 = regular turn (done, no tools).
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_event("SUMMARY")])
            .with_default(vec![done_event("ok")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(config, client).await;

    // Simulate a tool-intensive single-user session that has grown large:
    //   user(big) → assistant(tool) → tool → assistant(tool) → tool
    s.messages.push(big_user_message("u1", 8_000));
    s.messages.push(assistant_with_tool("a1", "call_1"));
    s.messages.push(tool_result("t1", "call_1", "result 1"));
    s.messages.push(assistant_with_tool("a2", "call_2"));
    s.messages.push(tool_result("t2", "call_2", "result 2"));

    // Snapshot the total character footprint BEFORE run adds its own messages.
    let chars_before: usize = s.messages.iter().map(|m| m.estimate_chars().len()).sum();

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    // Compaction must have fired: the big user message (8000 chars) was
    // replaced by a short summary, so the footprint must shrink dramatically.
    let chars_after: usize = s.messages.iter().map(|m| m.estimate_chars().len()).sum();
    assert!(
        chars_after < chars_before / 2,
        "transcript footprint must shrink by >50% after compaction (was {}, now {})",
        chars_before,
        chars_after
    );
    // First message is the synthetic summary.
    assert!(
        s.messages[0].synthetic,
        "first message must be the synthetic compaction summary"
    );
    assert!(
        s.messages[0]
            .text()
            .starts_with("[Conversation summary so far]"),
        "first message must carry the compaction-summary prefix"
    );
    // The big user message is gone (summarized into the head).
    assert!(
        !s.messages.iter().any(|m| m.id == "u1"),
        "the big user message must have been summarized away"
    );
    // At least one compaction LLM call happened (the summary call).
    let reqs = mock.requests();
    assert!(
        reqs.len() >= 2,
        "need >=2 calls (summary + turn), got {}",
        reqs.len()
    );
}

#[tokio::test]
async fn compaction_fires_with_real_default_config() {
    // End-to-end proof that compaction triggers and actually executes under the
    // REAL default config (no overrides): context_limit=128_000, threshold=80_000,
    // reserved=20_000, tail_turns=2. budget = min(80_000, 128_000 - 20_000) = 80_000.
    // Three 200k-char user messages (~150k tokens) vastly exceed that budget.
    let config = base_config(); // Config::default() except model for the test
    let (_dir, mut s) = session_with(config, client_with_default_done("unused")).await;

    // Assert the real defaults (no compaction field was overridden).
    assert_eq!(s.config.context_limit(), 128_000, "default context window");
    assert_eq!(s.config.compaction.context_threshold, 80_000, "default threshold");
    assert_eq!(s.config.compaction.reserved, 20_000, "default reserved");
    assert_eq!(s.config.compaction.tail_turns, 2, "default tail_turns");
    let budget = s
        .config
        .compaction
        .context_threshold
        .min(s.config.context_limit().saturating_sub(s.config.compaction.reserved));
    assert_eq!(budget, 80_000, "budget = min(threshold, limit - reserved)");

    // ~150k estimated tokens — well above the 80k budget.
    s.messages.push(big_user_message("u1", 200_000));
    s.messages.push(big_user_message("u2", 200_000));
    s.messages.push(big_user_message("u3", 200_000));

    // Layer 1: the trigger must fire BEFORE any run() call.
    assert!(should_compact(&s), "150k estimated tokens must trip the 80k budget");

    // Rebuild the session with a real mock script: summary call, then a
    // tool-less done so the loop terminates after one turn.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_event("SUMMARY")])
            .with_default(vec![done_event("done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir2, mut s) = session_with(base_config(), client).await;
    s.messages.push(big_user_message("u1", 200_000));
    s.messages.push(big_user_message("u2", 200_000));
    s.messages.push(big_user_message("u3", 200_000));

    let chars_before: usize = s.messages.iter().map(|m| m.estimate_chars().len()).sum();

    // run() adds a 4th user turn ("go"): u1,u2,u3,go -> 4 turns, tail_turns=2
    // -> split_index=2 -> u1+u2 get summarized, u3 retained.
    run(&mut s, "go".into(), |_| {}).await.unwrap();

    // Layer 2: compaction actually executed end-to-end.
    let chars_after: usize = s.messages.iter().map(|m| m.estimate_chars().len()).sum();
    assert!(
        chars_after < chars_before / 2,
        "footprint must shrink >50% (was {}, now {})",
        chars_before,
        chars_after
    );
    assert!(
        s.messages[0].synthetic,
        "first message must be the synthetic compaction summary"
    );
    assert!(
        s.messages[0]
            .text()
            .starts_with("[Conversation summary so far]"),
        "first message must carry the compaction-summary prefix"
    );
    assert!(
        !s.messages.iter().any(|m| m.id == "u1"),
        "u1 must have been summarized away"
    );
    assert!(
        !s.messages.iter().any(|m| m.id == "u2"),
        "u2 must have been summarized away"
    );
    assert!(
        s.messages.iter().any(|m| m.id == "u3"),
        "u3 must be retained (within the tail)"
    );

    let reqs = mock.requests();
    assert!(
        reqs.len() >= 2,
        "need >=2 calls (summary + turn), got {}",
        reqs.len()
    );
}
