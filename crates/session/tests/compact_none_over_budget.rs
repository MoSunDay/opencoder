//! Regression tests for the compaction `Ok(None)` over-budget path.
//!
//! `Ok(None)` means "should_compact fired but there is nothing to summarize"
//! — an empty or single-message transcript. Two follow-ups are possible:
//!
//! 1. The current transcript still FITS under the provider's hard context
//!    limit (e.g. the trigger was a stale reported usage from before a
//!    clear-context / plan→act handoff collapsed the transcript, or a single
//!    message that merely crosses the compaction *budget*). The runner must
//!    proceed to the LLM call — the compaction budget is a threshold, not a
//!    cap. Killing the run here is the "compaction failed: transcript exceeds
//!    context window but compaction found nothing to summarize" regression.
//!
//! 2. The transcript genuinely EXCEEDS the hard context limit. Nothing can be
//!    summarized away and the request is guaranteed to 400 — the runner must
//!    surface `Err` + `SessionEvent::Error` BEFORE dispatching any LLM call
//!    (falling through would send an oversized request that kills the
//!    session with a provider error).

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{compaction::should_compact, run, SessionEvent, SessionState};

/// Serializes tests that repoint `$HOME` so the host's global `~/.opencoder/
/// AGENTS.md` cannot perturb the compaction token estimate. Drop restores the
/// previous value. Mirrors the `ScopedHome` pattern in `compaction_and_model`.
struct ScopedHome {
    _guard: std::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    prev: Option<std::ffi::OsString>,
}

static HOME_MUTEX: Mutex<()> = Mutex::new(());

impl ScopedHome {
    fn new() -> ScopedHome {
        let guard = HOME_MUTEX.lock().unwrap();
        let prev = std::env::var_os("HOME");
        // A throwaway temp dir with no `.opencoder/AGENTS.md` → deterministic
        // (minimal) system-prompt token footprint for the estimate.
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

/// Find the first `SessionEvent::Error` payload in the collected stream.
fn first_error(events: &[SessionEvent]) -> Option<&str> {
    events.iter().find_map(|ev| match ev {
        SessionEvent::Error(m) => Some(m.as_str()),
        _ => None,
    })
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

/// A single oversized message that only crosses the compaction *budget* (tiny
/// `context_threshold`, default hard limit) must NOT kill the run: the
/// transcript fits under the provider limit, so the runner proceeds to the
/// LLM call unchanged. Previously this surfaced "compaction failed: transcript
/// exceeds context window but compaction found nothing to summarize".
#[tokio::test]
async fn over_budget_but_under_hard_limit_proceeds_to_llm_call() {
    let _home = ScopedHome::new();

    // Tiny threshold so the single user message (well over 10 estimated
    // tokens including the act system prompt) trips `should_compact`.
    // `tail_turns = 1` so a single-message transcript yields
    // `compaction_split == None` (the `Ok(None)` branch under test).
    let mut config = Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    };
    config.compaction.auto = true;
    config.compaction.context_threshold = 10;
    config.compaction.tail_turns = 1;

    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("done")]));
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").expect("act agent resolves");
    let mut s = SessionState::new(
        "compact-none-over-budget",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );

    // Sanity: a single oversized user message alone trips `should_compact`
    // under the tiny threshold, yet `compaction_split` has nothing to split.
    s.messages
        .push(opencoder_core::Message::user("u1", "x".repeat(2_000)));
    assert!(
        should_compact(&s),
        "precondition: single oversized message must trip should_compact"
    );
    assert_eq!(
        s.messages.len(),
        1,
        "precondition: exactly one message so compaction_split returns None"
    );

    // `run` records this user text as the transcript's single message before
    // entering run_loop, where compaction is checked at the top of the loop.
    // Push the message state we validated above out of the way: start the run
    // from an empty transcript so `run` itself contributes the lone message.
    s.messages.clear();
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_collector = events.clone();
    let outcome = run(&mut s, "x".repeat(2_000), move |ev| {
        if let Ok(mut g) = ev_collector.lock() {
            g.push(ev);
        }
    })
    .await;

    // 1) The run succeeds — no spurious compaction error for a request that
    //    fits under the hard context limit.
    assert!(
        outcome.is_ok(),
        "run must succeed when the single-message transcript fits under the hard limit, got {outcome:?}"
    );

    // 2) The LLM call was dispatched exactly once (the normal turn).
    assert_eq!(
        mock.call_count(),
        1,
        "the fitting request must reach the LLM instead of being killed by compaction"
    );

    // 3) No compaction error event was emitted.
    let collected = events.lock().unwrap().clone();
    assert!(
        first_error(&collected).is_none(),
        "no SessionEvent::Error expected, got: {collected:?}"
    );
}

/// A single message that genuinely EXCEEDS the hard context limit cannot be
/// summarized away — the runner must surface `Err` + an Error event BEFORE
/// any LLM call instead of dispatching a guaranteed-400 request.
#[tokio::test]
async fn over_hard_limit_with_nothing_to_compact_errors_before_llm_call() {
    let _home = ScopedHome::new();

    // Tiny hard context limit: the estimate (system prompt + 2k-char message)
    // is far beyond it, so the request is guaranteed to be rejected.
    let mut config = Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    };
    config.context_limit = Some(50);
    config.compaction.auto = true;
    config.compaction.context_threshold = 10;
    config.compaction.tail_turns = 1;

    // NO script and NO default: any LLM call would be recorded then fail with
    // "mock exhausted". The assertion `call_count() == 0` proves the call was
    // never made.
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").expect("act agent resolves");
    let mut s = SessionState::new(
        "compact-none-over-budget",
        agent,
        config,
        client,
        dir.path().to_path_buf(),
    );

    // Sanity: a single oversized user message alone trips `should_compact`
    // under the tiny threshold, yet `compaction_split` has nothing to split.
    s.messages
        .push(opencoder_core::Message::user("u1", "x".repeat(2_000)));
    assert!(
        should_compact(&s),
        "precondition: single oversized message must trip should_compact"
    );
    assert_eq!(
        s.messages.len(),
        1,
        "precondition: exactly one message so compaction_split returns None"
    );

    s.messages.clear();
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_collector = events.clone();
    let outcome = run(&mut s, "x".repeat(2_000), move |ev| {
        if let Ok(mut g) = ev_collector.lock() {
            g.push(ev);
        }
    })
    .await;

    let collected = events.lock().unwrap().clone();

    // 1) The runner must surface an `Err` instead of dispatching a
    //    guaranteed-400 request.
    assert!(
        outcome.is_err(),
        "run must return Err when the transcript exceeds the hard limit with nothing to compact, got {outcome:?}"
    );

    // 2) A `SessionEvent::Error` must be emitted naming the over-budget /
    // cannot-compact condition (NOT a downstream "mock exhausted" / 400).
    let err_msg = first_error(&collected)
        .unwrap_or_else(|| panic!("expected a SessionEvent::Error event, got: {:?}", collected));
    assert!(
        err_msg.contains("context window"),
        "error must explain the over-budget/no-compact condition, got: {err_msg}"
    );
    assert!(
        err_msg.contains("compaction failed"),
        "error must carry the 'compaction failed' prefix, got: {err_msg}"
    );

    // 3) No LLM call was ever dispatched — the oversized request never left
    // the runner.
    assert_eq!(
        mock.call_count(),
        0,
        "no LLM call must be made when the transcript cannot be compacted"
    );
}
