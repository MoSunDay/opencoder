//! P1 functional tests for compaction + model selection behavior.
//! All driven by MockChatClient — zero network, fully deterministic.

use std::path::Path;
use std::sync::{Arc, Mutex};

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
            ..Default::default()
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
    let _home = ScopedHome::new();
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
    let _home = ScopedHome::new();
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
        // ~1125 estimated transcript tokens. With the act system-prompt
        // footprint (~425 tokens) the total sits under usable=1800 but well
        // over usable=900, so reserved is the sole discriminant.
        s.messages.push(big_user_message("u1", 4_500));
        should_compact(&s)
    }
    // reserved=200 → usable=1800 → 1400 < 1800 → no compact
    assert!(!mk(200), "with small reserved, 1400 tokens should NOT trip");
    // reserved=1100 → usable=900 → 1400 >= 900 → compact
    assert!(mk(1_100), "with large reserved, 1400 tokens MUST trip");
}

#[tokio::test]
async fn small_model_is_used_for_compaction_summary_call() {
    let _home = ScopedHome::new();
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
    let _home = ScopedHome::new();
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
    let _home = ScopedHome::new();
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
        ..Default::default()
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
        ..Default::default()
    };
    assert!(
        should_compact(&s),
        "must trigger when input_tokens exceeds budget"
    );
}

#[tokio::test]
async fn compaction_fires_in_tool_intensive_single_user_session() {
    let _home = ScopedHome::new();
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
    let _home = ScopedHome::new();
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

#[tokio::test]
async fn compaction_fires_when_over_budget_but_few_turns() {
    let _home = ScopedHome::new();
    // REGRESSION for the silent no-op bug: an over-budget transcript whose
    // turn count is <= tail_turns used to bail out of compaction entirely
    // (split_index returned 0 -> compact() returned Ok(None)), shipping the
    // full oversized context to the model with no summary produced. The
    // compaction_split fallback must still compress it.
    let mut config = base_config();
    config.context_limit = Some(2_000);
    config.compaction.reserved = 100;
    config.compaction.context_threshold = 10_000; // usable (=1900) binds the budget
    config.compaction.tail_turns = 2; // the transcript below yields only 2 turns

    // A single big turn (~5000 estimated tokens >> 1900 budget). After run()
    // prepends the "go" prompt the transcript has exactly 2 turn-starts
    // (u1 at index 0, "go" at index 1) -- i.e. <= tail_turns(2), the exact
    // shape that used to no-op.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_event("SUMMARY")]) // compaction summarize call
            .with_default(vec![done_event("done")]),  // subsequent turn(s)
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (_dir, mut s) = session_with(config, client).await;
    s.messages.push(big_user_message("u1", 20_000));

    assert_eq!(
        s.config.compaction.tail_turns, 2,
        "precondition: tail_turns=2 so a 2-turn transcript hits the fallback"
    );

    let chars_before: usize = s.messages.iter().map(|m| m.estimate_chars().len()).sum();
    run(&mut s, "go".into(), |_| {}).await.unwrap();

    // Compaction MUST have executed rather than the old no-op: the big message
    // is summarized away and a synthetic summary leads the transcript.
    let chars_after: usize = s.messages.iter().map(|m| m.estimate_chars().len()).sum();
    assert!(
        chars_after < chars_before,
        "compaction must shrink the transcript (was {}, now {})",
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
        "the big user message must have been summarized away (was a no-op pre-fix)"
    );

    // The summarize LLM call happened.
    let reqs = mock.requests();
    assert!(
        reqs.len() >= 2,
        "need >=2 calls (summary + turn), got {}",
        reqs.len()
    );
}

/// Serialize HOME-sensitive tests within this binary.
static COMPACT_HOME_MUTEX: Mutex<()> = Mutex::new(());

fn with_home<R>(home: &Path, f: impl FnOnce() -> R) -> R {
    let _guard = COMPACT_HOME_MUTEX.lock().unwrap();
    let old = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    let r = f();
    match old {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    r
}

/// RAII guard that pins `HOME` to a clean empty temp dir and serializes
/// against `with_home` (and the differential global-file test) via
/// `COMPACT_HOME_MUTEX`. Drop restores the previous `HOME`.
///
/// Required for the async tests below, whose bodies `.await` and therefore
/// cannot be wrapped by `with_home` (a sync closure). `should_compact` reads
/// the global `~/.opencoder/AGENTS.md` through `HOME`, so any test that calls
/// `should_compact` (directly or via `run`) must hold this guard to stay
/// deterministic — immune both to the differential test's `HOME` mutation and
/// to a real global file on the host. Pinning to a clean dir (not just
/// locking) preserves the original "no global file" semantics.
struct ScopedHome {
    _guard: std::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    prev: Option<std::ffi::OsString>,
}

impl ScopedHome {
    fn new() -> ScopedHome {
        let guard = COMPACT_HOME_MUTEX.lock().unwrap();
        let prev = std::env::var_os("HOME");
        let dir = tempfile::TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        ScopedHome {
            _guard: guard,
            _dir: dir,
            prev,
        }
    }
}

impl Drop for ScopedHome {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}

#[test]
fn global_agents_md_excluded_from_compaction_budget() {
    // A large global ~/.opencoder/AGENTS.md must NOT affect the compaction
    // decision. We prove this differentially: `should_compact` must return
    // the same value whether or not the global file is present. The transcript
    // is sized so that, IF the global file were counted, the estimate would
    // jump far over the 1800-token budget and trip compaction.
    let home = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(home.path().join(".opencoder")).unwrap();
    // 100k chars ≈ 25k tokens — far above the 1800 budget if it were counted.
    std::fs::write(
        home.path().join(".opencoder").join("AGENTS.md"),
        "g".repeat(100_000),
    )
    .unwrap();

    let mut config = base_config();
    config.context_limit = Some(2_000);
    config.compaction.reserved = 200;
    config.compaction.context_threshold = 10_000; // usable(1800) binds as budget

    let global_path = home.path().join(".opencoder").join("AGENTS.md");

    with_home(home.path(), || {
        let dir = tempfile::tempdir().unwrap();
        let agent = resolve_agent("act").unwrap();
        let client: Arc<dyn ChatStream> = client_with_default_done("ok");
        let mut s = SessionState::new("g", agent, config.clone(), client, dir.path().to_path_buf());
        // ~1125 estimated transcript tokens — comfortably under the 1800 budget
        // with headroom for the (act) system prompt. The 100k-char global file
        // (~25k tokens) would still blow the budget if it were ever counted.
        s.messages.push(big_user_message("u1", 4_500));

        // With the global file present: excluded → must NOT trip.
        let with_global = should_compact(&s);
        // Remove the global file and re-evaluate on the same session.
        std::fs::remove_file(&global_path).unwrap();
        let without_global = should_compact(&s);

        assert!(
            !with_global,
            "global agents.md must be excluded so the budget is not blown"
        );
        assert_eq!(
            with_global, without_global,
            "global agents.md must not change the compaction decision"
        );
    });
}
