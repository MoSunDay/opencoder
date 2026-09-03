//! Sidecar loop integration tests (TUI `/sidecar`, Phase 1 session side).
//!
//! Locks the three contract pillars:
//! 1. **snapshot-in, memory continuation**: the loop starts from the parent
//!    transcript and follow-up turns see prior Q/A (no store round-trip);
//! 2. **zero persistence**: no sidecar session row, no message rows, no
//!    sidecar event rows anywhere in the store;
//! 3. **cost lands on the main task**: each sidecar LLM round reaches the
//!    parent event stream as a *bare* `LlmUsage` (persistable), while sidecar
//!    content frames are filtered out by the `EventSink::push` gate.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{
    new_conv, parse_sidecar_question, run_sidecar_turn, spawn_event_flusher, SessionEvent,
    SessionState,
};
use opencoder_store::{LibsqlStore, Store};

const PARENT: &str = "parent-1";

fn config() -> Config {
    Config {
        model: "mock-model".into(),
        ..Config::default()
    }
}

fn done(text: &str, total: u64, input: u64, output: u64) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::<CompletedToolCall>::new(),
        usage: Some(Usage {
            total_tokens: total,
            input_tokens: input,
            output_tokens: output,
            ..Default::default()
        }),
    }
}

/// Parent session with a real memory store + mock client. `record` auto-creates
/// the session row, so seeded history is durably countable.
async fn parent_session() -> (SessionState, Arc<dyn Store>, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let agent = resolve_agent("act").expect("act agent registered");
    let session = SessionState::new(
        PARENT,
        agent,
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        std::env::current_dir().unwrap(),
    )
    .with_store(store.clone());
    (session, store, mock)
}

/// Two history turns so the snapshot has real parent content.
async fn seed_history(parent: &mut SessionState) {
    parent
        .record(Message::user("u1", "refactor the parser module"))
        .await;
    let mut a = Message::assistant("a1");
    a.blocks = vec![ContentBlock::text("phase one complete: tokenizer")];
    parent.record(a).await;
}

/// Shared event collector passed as the parent-side `on_event`.
fn collector() -> (
    Arc<Mutex<Vec<SessionEvent>>>,
    impl FnMut(SessionEvent) + Send,
) {
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let cb = move |ev: SessionEvent| sink.lock().unwrap().push(ev);
    (events, cb)
}

/// `/sidecar` recognition: bare, with question, spaced - but never a
/// look-alike token and never another command.
#[test]
fn parse_sidecar_question_cases() {
    assert_eq!(parse_sidecar_question("/sidecar").unwrap(), "");
    assert_eq!(parse_sidecar_question("/sidecar now?").unwrap(), "now?");
    assert_eq!(
        parse_sidecar_question("  /sidecar   how far   ").unwrap(),
        "how far"
    );
    // No word boundary -> a different command, not ours.
    assert!(parse_sidecar_question("/sidecarX").is_none());
    assert!(parse_sidecar_question("/sidecaring").is_none());
    // Not a sidecar command at all.
    assert!(parse_sidecar_question("hello").is_none());
    assert!(parse_sidecar_question("/plan now").is_none());
}

/// One turn: answer produced, parent untouched, store untouched, cost
/// forwarded bare, content frames wrapped, summary frame emitted.
#[tokio::test]
async fn sidecar_turn_answers_without_persisting() {
    let (mut parent, store, mock) = parent_session().await;
    seed_history(&mut parent).await;
    let parent_mem_before = parent.messages.len();
    let parent_store_before = store.load_messages(PARENT).await.unwrap().len();

    mock.queue_script(vec![done("2 of 5 todos done", 1234, 1000, 234)]);

    let mut conv = new_conv(&parent).await.unwrap();
    assert!(conv.id.starts_with("sidecar-"), "got {}", conv.id);
    // Snapshot: the child starts with the parent transcript clone.
    assert_eq!(conv.child.messages.len(), parent_mem_before);
    let child_before = conv.child.messages.len();

    let (events, cb) = collector();
    let mut cb = cb;
    let turn = run_sidecar_turn(&mut conv, "progress?", &mut cb)
        .await
        .unwrap();

    // ---- Turn summary ----
    assert!(turn.ok, "turn must succeed");
    assert_eq!(turn.total_tokens, 1234);
    assert_eq!(turn.rounds, 1);
    assert!(
        turn.answer.contains("2 of 5"),
        "answer was: {}",
        turn.answer
    );

    // ---- In-memory continuation, zero store writes ----
    assert_eq!(
        conv.child.messages.len(),
        child_before + 2,
        "user+assistant"
    );
    assert_eq!(parent.messages.len(), parent_mem_before);
    assert_eq!(
        store.load_messages(PARENT).await.unwrap().len(),
        parent_store_before
    );
    assert!(
        store.get_session(&conv.id).await.unwrap().is_none(),
        "sidecar must not create a session row"
    );
    assert!(
        store.events_after(&conv.id, 0).await.unwrap().is_empty(),
        "sidecar must not persist events under its own id"
    );

    let evs = events.lock().unwrap().clone();
    // Bare LlmUsage: exactly one, at top level (parent-task accounting).
    let bare_usage = evs
        .iter()
        .filter(|e| {
            matches!(
                e,
                SessionEvent::LlmUsage {
                    total_tokens: 1234,
                    ..
                }
            )
        })
        .count();
    assert_eq!(bare_usage, 1, "exactly one bare LlmUsage, got {bare_usage}");
    // Content frames are wrapped in SidecarChild...
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::SidecarChild { .. })),
        "expected at least one SidecarChild frame"
    );
    // ...and never carry a bare LlmUsage / Done / Error inside.
    for ev in &evs {
        if let SessionEvent::SidecarChild { ev: inner, .. } = ev {
            assert!(
                !matches!(
                    inner.as_ref(),
                    SessionEvent::LlmUsage { .. } | SessionEvent::Done | SessionEvent::Error(_)
                ),
                "SidecarChild must not wrap {inner:?}"
            );
        }
    }
    assert!(!evs.iter().any(|e| matches!(e, SessionEvent::Done)));
    // Summary frame present with the same accounting as the returned turn.
    assert!(evs.iter().any(|e| matches!(
        e,
        SessionEvent::SidecarTurn {
            ok: true,
            total_tokens: 1234,
            rounds: 1,
            ..
        }
    )));
}

/// Follow-up on the same conv: the second request must carry the first Q/A
/// round (in-memory history continuation, no re-snapshot).
#[tokio::test]
async fn sidecar_followup_sees_prior_turn() {
    let (parent, _store, mock) = parent_session().await;
    mock.queue_script(vec![done("answer one", 100, 80, 20)]);
    mock.queue_script(vec![done("answer two", 150, 120, 30)]);

    let mut conv = new_conv(&parent).await.unwrap();
    let (e1, cb1) = collector();
    let mut cb1 = cb1;
    run_sidecar_turn(&mut conv, "first question", &mut cb1)
        .await
        .unwrap();
    let (e2, cb2) = collector();
    let mut cb2 = cb2;
    run_sidecar_turn(&mut conv, "second question", &mut cb2)
        .await
        .unwrap();

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "each turn is exactly one LLM call");
    let second = serde_json::to_string(&reqs[1].messages).unwrap();
    assert!(
        second.contains("first question"),
        "second request lost the first Q: {second}"
    );
    assert!(
        second.contains("answer one"),
        "second request lost the first A: {second}"
    );
    assert!(
        second.contains("second question"),
        "second request lost the new question: {second}"
    );
    // And the turn summaries agree on per-turn cost.
    let (all1, all2) = (e1.lock().unwrap().clone(), e2.lock().unwrap().clone());
    let all = all1.iter().chain(all2.iter());
    assert!(all.clone().any(|e| matches!(
        e,
        SessionEvent::SidecarTurn {
            ok: true,
            total_tokens: 150,
            ..
        }
    )));
}

/// A control command is navigation for the parent session, never a sidecar
/// question: refused without a single LLM call.
#[tokio::test]
async fn sidecar_rejects_control_question() {
    let (parent, _store, mock) = parent_session().await;
    let mut conv = new_conv(&parent).await.unwrap();
    let (events, cb) = collector();
    let mut cb = cb;
    let turn = run_sidecar_turn(&mut conv, "/plan", &mut cb).await.unwrap();

    assert!(!turn.ok);
    assert_eq!(
        mock.call_count(),
        0,
        "control commands must not hit the LLM"
    );
    assert_eq!(conv.child.messages.len(), 0, "child transcript untouched");
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, SessionEvent::SidecarTurn { ok: false, .. })));
}

/// The `EventSink::push` gate drops sidecar frames entirely while the bare
/// `LlmUsage` (cost accounting) still persists.
#[tokio::test]
async fn event_sink_filters_sidecar_frames() {
    // Real parent row first: append_events carries an FK on sessions, and the
    // flusher drops failed writes (warn-only), so the row must exist to prove
    // the *filter* (not an FK failure) keeps sidecar frames out.
    let (mut parent, store, _mock) = parent_session().await;
    seed_history(&mut parent).await;
    let (sink, flusher) = spawn_event_flusher(Some(store.clone()), PARENT.into());

    let _ = sink.push(&SessionEvent::SidecarStart {
        id: "sidecar-x".into(),
        question: "q".into(),
    });
    let _ = sink.push(&SessionEvent::SidecarChild {
        id: "sidecar-x".into(),
        ev: Box::new(SessionEvent::TextDelta("t".into())),
    });
    let _ = sink.push(&SessionEvent::SidecarTurn {
        id: "sidecar-x".into(),
        ok: true,
        answer: "a".into(),
        elapsed_ms: 1,
        total_tokens: 5,
        rounds: 1,
    });
    // The accounting channel is NOT a sidecar frame: it must persist.
    let _ = sink.push(&SessionEvent::LlmUsage {
        total_tokens: 1234,
        input_tokens: 1000,
        output_tokens: 234,
    });

    drop(sink);
    flusher.await.unwrap();

    let evs = store.events_after(PARENT, 0).await.unwrap();
    assert_eq!(evs.len(), 1, "only the bare LlmUsage may persist: {evs:?}");
    assert_eq!(evs[0].sse_kind.as_deref(), Some("llm_usage"));
}

/// A provider failure inside the sidecar stays inside: no bare `Error` on the
/// parent stream, failure surfaced via `SidecarTurn { ok: false }` instead.
#[tokio::test]
async fn sidecar_child_error_does_not_leak() {
    let (parent, _store, mock) = parent_session().await;
    mock.queue_script(vec![LlmEvent::Error("boom: provider down".into())]);

    let mut conv = new_conv(&parent).await.unwrap();
    let (events, cb) = collector();
    let mut cb = cb;
    let turn = run_sidecar_turn(&mut conv, "status?", &mut cb)
        .await
        .unwrap();

    assert!(!turn.ok);
    assert!(
        turn.answer.contains("boom"),
        "answer should carry the failure reason: {}",
        turn.answer
    );
    let evs = events.lock().unwrap().clone();
    assert!(
        !evs.iter().any(|e| matches!(e, SessionEvent::Error(_))),
        "a bare Error must not leak to the parent stream: {evs:?}"
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::LlmUsage { .. })),
        "a failed round has no usage to account"
    );
    assert!(evs
        .iter()
        .any(|e| matches!(e, SessionEvent::SidecarTurn { ok: false, .. })));
}

/// A parent snapshot that is already over the compaction threshold must NOT
/// make the sidecar compact: the child transcript is a borrowed snapshot, not
/// this loop's durable history. Compacting would (a) pay an extra LLM summary
/// round per question and (b) replace the snapshot with a summary, so follow-up
/// turns would lose the very context they exist to answer from.
#[tokio::test]
async fn sidecar_never_compacts_over_threshold_snapshot() {
    let (mut parent, _store, mock) = parent_session().await;
    // Lower the threshold BEFORE seeding so the seeded history alone trips it:
    // the test must prove the snapshot (not something this turn adds) is the
    // trigger — otherwise it would pass vacuously.
    parent.config.compaction.context_threshold = 10;
    seed_history(&mut parent).await;
    assert!(
        opencoder_session::compaction::should_compact(&parent),
        "precondition: the parent snapshot alone must trip the compaction gate"
    );
    let parent_before_len = parent.messages.len();
    let parent_texts: Vec<String> = parent.messages.iter().map(|m| m.text()).collect();

    mock.queue_script(vec![done("phase one done: tokenizer", 111, 100, 11)]);

    let mut conv = new_conv(&parent).await.unwrap();
    assert!(
        !conv.child.config.compaction.auto,
        "sidecar child must disable auto compaction"
    );

    let (events, cb) = collector();
    let mut cb = cb;
    let turn = run_sidecar_turn(&mut conv, "progress?", &mut cb)
        .await
        .unwrap();

    assert!(turn.ok, "turn must succeed");
    assert_eq!(
        turn.rounds, 1,
        "a compaction round would consume a second LLM round"
    );

    let evs = events.lock().unwrap().clone();
    for ev in &evs {
        if let SessionEvent::SidecarChild { ev: inner, .. } = ev {
            assert!(
                !matches!(
                    inner.as_ref(),
                    SessionEvent::Compaction(_) | SessionEvent::TranscriptReset(_)
                ),
                "sidecar must not emit {inner:?}"
            );
        }
        assert!(
            !matches!(
                ev,
                SessionEvent::Compaction(_) | SessionEvent::TranscriptReset(_)
            ),
            "sidecar must not emit {ev:?}"
        );
    }

    // The snapshot survived intact: the parent prefix is untouched, the turn
    // only appended the Q/A pair.
    assert_eq!(conv.child.messages.len(), parent_before_len + 2);
    for (i, want) in parent_texts.iter().enumerate() {
        assert_eq!(
            &conv.child.messages[i].text(),
            want,
            "snapshot message {i} must survive verbatim"
        );
    }
}

/// Bug-fix #7: the sidecar is exempt from the manual-compaction hard-limit
/// gate. Its transcript is a borrowed parent snapshot with `compaction.auto`
/// deliberately forced off (`runner/sidecar.rs::build_sidecar_conv`), so with
/// the parent near its context window the gate aborted EVERY question with a
/// "/compact" hint that is unexecutable in sidecar context - and the snapshot
/// can never shrink, so it failed permanently instead of surfacing the real
/// provider error.
#[tokio::test]
async fn sidecar_answers_when_parent_snapshot_exceeds_hard_limit() {
    let (mut parent, _store, mock) = parent_session().await;
    seed_history(&mut parent).await;
    // Absurdly small physical window: even the system prompt exceeds 1 token.
    // The child clones this config in `new_conv` and forces auto=false, so
    // `exceeds_hard_limit(child)` is trivially true.
    parent.config.context_limit = Some(1);

    mock.queue_script(vec![done("still answering", 10, 5, 5)]);

    let mut conv = new_conv(&parent).await.unwrap();
    let (events, cb) = collector();
    let mut cb = cb;
    let turn = run_sidecar_turn(&mut conv, "progress?", &mut cb)
        .await
        .unwrap();

    assert!(turn.ok, "turn must succeed, answer was: {}", turn.answer);
    assert!(
        turn.answer.contains("still answering"),
        "answer was: {}",
        turn.answer
    );
    // No unexecutable "/compact" hint may reach the parent-side stream.
    let evs = events.lock().unwrap().clone();
    assert!(
        !evs.iter()
            .any(|e| matches!(e, SessionEvent::Error(m) if m.contains("/compact"))),
        "unexpected /compact hint in parent stream: {evs:?}"
    );
}
