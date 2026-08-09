//! Regression test for the compaction `Ok(None)` over-budget path.
//!
//! Bug: `run_loop`'s compaction block previously treated `Ok(None)` —
//! "should_compact fired but compaction_split found nothing to summarize
//! (single oversized message / empty head)" — as a no-op (`last_err = None`,
//! `break`). It then fell through to `run_one_llm_call` with a transcript
//! that was already over budget, guaranteeing a context-length 400 from the
//! provider that kills the session.
//!
//! Fix: that branch now records `last_err = Some(anyhow!(...))`, which makes
//! the surrounding `if let Some(e) = last_err` emit `SessionEvent::Error` and
//! `return Err(e)` BEFORE any LLM call.
//!
//! This test reconstructs the exact trigger — a single user message under a
//! tiny `context_threshold` so `should_compact` is true yet `compaction_split`
//! returns `None` — and proves the runner surfaces an `Err` (with an Error
//! event) instead of dispatching an oversized request to the LLM. The mock is
//! armed with NO script at all: `call_count() == 0` after the run is the
//! proof that the oversized request was never sent. (Were the bug present, the
//! LLM call would land first and `call_count()` would be `1`.)

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, MockChatClient};
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

#[tokio::test]
async fn over_budget_with_nothing_to_compact_errors_before_llm_call() {
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

    // `run` records this user text as the transcript's single message before
    // entering run_loop, where compaction is checked at the top of the loop.
    // Push the message state we validated above out of the way: start the run
    // from an empty transcript so `run` itself contributes the lone message.
    s.messages.clear();
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_collector = events.clone();
    let outcome = run(
        &mut s,
        "x".repeat(2_000),
        move |ev| {
            if let Ok(mut g) = ev_collector.lock() {
                g.push(ev);
            }
        },
    )
    .await;

    let collected = events.lock().unwrap().clone();

    // 1) The runner must surface an `Err` instead of falling through.
    assert!(
        outcome.is_err(),
        "run must return Err when over budget with nothing to compact, got {outcome:?}"
    );

    // 2) A `SessionEvent::Error` must be emitted naming the over-budget /
    // cannot-compact condition (NOT a downstream "mock exhausted" / 400).
    let err_msg = first_error(&collected).unwrap_or_else(|| {
        panic!(
            "expected a SessionEvent::Error event, got: {:?}",
            collected
        )
    });
    assert!(
        err_msg.contains("context window"),
        "error must explain the over-budget/no-compact condition, got: {err_msg}"
    );
    assert!(
        err_msg.contains("compaction failed"),
        "error must carry the 'compaction failed' prefix, got: {err_msg}"
    );

    // 3) No LLM call was ever dispatched — the oversized request never left
    // the runner. This is the core regression guard: were the bug present, the
    // fall-through LLM call would land here and `call_count()` would be `1`.
    assert_eq!(
        mock.call_count(),
        0,
        "no LLM call must be made when the transcript cannot be compacted"
    );
}
