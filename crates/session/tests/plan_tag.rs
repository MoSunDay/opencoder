//! Integration test: the plan-mode read-only reminder tag is injected at all
//! three runner entry points — direct prompt, steer promotion, and queue drain.
//!
//! The pure unit tests in `crates/session/src/lib.rs` cover
//! `maybe_tag_plan_prompt` in isolation. This file drives the *real* runner
//! (`run`) so the three call sites in `runner/mod.rs` are exercised end-to-end:
//!
//! 1. `direct_prompt_tags_only_after_first` — injection point 1 (direct-prompt
//!    branch): the first requirement is left clean; every subsequent
//!    requirement is suffixed with the read-only tag, and the model request
//!    body reflects the same.
//! 2. `steer_prompt_tagged_after_first` — injection point 2 (turn-boundary
//!    steer promotion): a steer admitted before the run is tagged because the
//!    kickoff prompt already advanced `plan_input_count`.
//! 3. `queued_prompt_tagged_after_first` — injection point 3 (idle queue
//!    drain): a queued follow-up drained after the kickoff turn is tagged when
//!    it is replayed as a real user turn.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_session::{run, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

/// The exact read-only reminder `maybe_tag_plan_prompt` appends (without the
/// leading newline, which is added at the call site).
const TAG: &str = "（当前处于只读的 plan 模式，聚焦计划生成）";

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Create the session row so FK-backed input admission succeeds.
async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: id.into(),
            agent: Some("plan".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Admit a store-backed input (steer or queue) for `session_id`.
async fn admit(store: &Arc<dyn Store>, session_id: &str, id: &str, prompt: &str, delivery: Delivery) {
    store
        .admit_input(&SessionInput {
            seq: None,
            id: id.into(),
            session_id: session_id.into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();
}

/// Build a plan-mode session wired to a store + mock client.
fn plan_session(
    store: Arc<dyn Store>,
    mock: Arc<dyn ChatStream>,
    id: &str,
) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let s = SessionState::new(
        id,
        resolve_agent("plan").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .with_store(store)
    .mark_session_created();
    (dir, s)
}

/// Text of the last non-synthetic (user-typed) user message.
fn last_real_user_text(session: &SessionState) -> String {
    session
        .messages
        .iter()
        .rfind(|m| m.role == Role::User && !m.synthetic)
        .map(|m| m.text())
        .unwrap_or_default()
}

/// Text of the first user message (the kickoff / initial prompt).
fn kickoff_text(session: &SessionState) -> String {
    session
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| m.text())
        .unwrap_or_default()
}

/// Texts of every user message after the kickoff (steers / drained queue).
fn promoted_user_texts(session: &SessionState) -> Vec<String> {
    session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .skip(1)
        .map(|m| m.text())
        .collect()
}

/// True if any lowered message in the request carries the plan-mode tag.
fn request_has_tag(req: &ChatRequest) -> bool {
    req.messages.iter().any(|m| m.to_string().contains(TAG))
}

/// Injection point 1 — direct prompt (`runner/mod.rs` direct-prompt branch).
/// The first requirement is never tagged; the second is, and the tag reaches
/// the model only on the second turn's request.
#[tokio::test]
async fn direct_prompt_tags_only_after_first() {
    let store = mem_store().await;
    seed(&store, "direct-sess").await;
    // Keep the concrete mock so `.requests()` stays observable; cast only at
    // the call site so the runner receives its `dyn ChatStream` view.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("plan reply 1")])
            .push_script(vec![done_turn("plan reply 2")]),
    );
    let (_dir, mut s) = plan_session(store, mock.clone(), "direct-sess");

    // First requirement: clean (plan_input_count 0 -> 1).
    run(&mut s, "first requirement".into(), |_| {})
        .await
        .unwrap();
    assert_eq!(last_real_user_text(&s), "first requirement");
    assert_eq!(s.plan_input_count, 1);

    // Second requirement: tagged (plan_input_count 1 -> 2).
    run(&mut s, "second requirement".into(), |_| {})
        .await
        .unwrap();
    let second = last_real_user_text(&s);
    assert!(
        second.starts_with("second requirement"),
        "unexpected second prompt: {second}"
    );
    assert!(second.contains(TAG), "second prompt must carry tag: {second}");
    assert_eq!(s.plan_input_count, 2);

    // The model saw the tag only from the second turn onward.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "one LLM call per requirement");
    assert!(!request_has_tag(&reqs[0]), "turn-1 request must not carry tag");
    assert!(request_has_tag(&reqs[1]), "turn-2 request must carry tag");
}

/// Injection point 2 — turn-boundary steer promotion. A steer admitted before
/// the run is promoted at the first iteration top, after the kickoff prompt
/// already advanced the counter, so the steer text is tagged.
#[tokio::test]
async fn steer_prompt_tagged_after_first() {
    let store = mem_store().await;
    seed(&store, "steer-sess").await;
    admit(
        &store,
        "steer-sess",
        "steer-1",
        "steered requirement",
        Delivery::Steer,
    )
    .await;
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn("plan reply")]));
    let (_dir, mut s) = plan_session(store, mock.clone(), "steer-sess");

    // Kickoff (count 0 -> 1, untagged) + steer promoted at the first boundary
    // (count 1 -> 2, tagged) within a single run.
    run(&mut s, "kickoff".into(), |_| {}).await.unwrap();

    assert_eq!(kickoff_text(&s), "kickoff");
    let promoted = promoted_user_texts(&s);
    assert_eq!(promoted.len(), 1, "exactly one steer promoted");
    assert!(promoted[0].contains(TAG), "steer must be tagged: {}", promoted[0]);
    assert_eq!(s.plan_input_count, 2);

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1, "single turn absorbs the steer");
    assert!(
        request_has_tag(&reqs[0]),
        "the promoted steer must reach the model"
    );
}

/// Injection point 3 — idle queue drain. A queued follow-up is drained after
/// the kickoff turn completes; since the kickoff already advanced the counter,
/// the queued prompt is tagged when replayed as a real user turn.
#[tokio::test]
async fn queued_prompt_tagged_after_first() {
    let store = mem_store().await;
    seed(&store, "queue-sess").await;
    admit(
        &store,
        "queue-sess",
        "queue-1",
        "queued requirement",
        Delivery::Queue,
    )
    .await;
    // Turn 1: kickoff (untagged) -> idle -> drain queue (tagged) -> turn 2.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("kickoff reply")])
            .push_script(vec![done_turn("queued reply")]),
    );
    let (_dir, mut s) = plan_session(store, mock.clone(), "queue-sess");

    run(&mut s, "kickoff".into(), |_| {}).await.unwrap();

    assert_eq!(kickoff_text(&s), "kickoff");
    let promoted = promoted_user_texts(&s);
    assert_eq!(promoted.len(), 1, "exactly one queued item drained");
    assert!(
        promoted[0].contains(TAG),
        "queued prompt must be tagged: {}",
        promoted[0]
    );
    assert_eq!(s.plan_input_count, 2);

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "kickoff turn + queued turn");
    assert!(!request_has_tag(&reqs[0]), "kickoff turn must not carry tag");
    assert!(request_has_tag(&reqs[1]), "queued turn must carry tag");
}
