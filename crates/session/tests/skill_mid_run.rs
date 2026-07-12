//! Skill activation mid-run: when a skill is set via the shared `Arc<Mutex>`
//! while a session is running (between turns), the next turn's system prompt
//! must include the skill body.
//!
//! Before the fix, `skill_prompt` was `Option<String>` updated through the
//! cmd channel (`UiCmd::SetSkill`). While `run_loop` was executing, the
//! worker could not process cmd-channel messages until `run_loop` returned,
//! so the skill never reached the turn that needed it. The fix makes
//! `skill_prompt` an `Arc<Mutex<Option<String>>>` so the TUI can update it
//! directly, and `run_one_llm_call` reads the latest value each turn.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A turn that calls `bash` (so the loop continues), carrying `n` in usage.
fn bash_turn(n: u32) -> LlmEvent {
    LlmEvent::Completed {
        text: format!("turn-{n}"),
        tool_calls: vec![CompletedToolCall {
            id: format!("tu{n}"),
            name: "bash".into(),
            input: serde_json::json!({"command": "true"}),
        }],
        usage: Some(Usage {
            input_tokens: 10 * n as u64,
            output_tokens: 1,
            total_tokens: 10 * n as u64 + 1,
        }),
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Extract the system message content from a ChatRequest's messages.
fn system_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

/// Create the session row so input admission (FK) succeeds before the run.
async fn seed_session(store: &Arc<dyn Store>) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "skill-mid-run".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();
}

/// When a skill is set via the shared `Arc<Mutex>` during turn 1's tool
/// execution, turn 2's system prompt must include the skill body — even
/// though turn 1's system prompt did not.
///
/// Flow:
/// 1. Turn 1: bash tool call → ToolStart event fires → skill is set via Arc
/// 2. Turn boundary: a pre-admitted steer is promoted into history
/// 3. Turn 2: `run_one_llm_call` reads `skill_prompt_cloned()` → finds the skill
/// 4. Turn 2: done (no tool calls) → idle → Done
#[tokio::test]
async fn skill_set_mid_run_appears_in_next_turn_system_prompt() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn(1)])
            .push_script(vec![done_turn("done")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "skill-mid-run",
        agent,
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    seed_session(&store).await;

    // Admit a steer BEFORE the run so it's promoted at the turn boundary
    // between turn 1 and turn 2 — guaranteeing a second LLM call.
    store
        .admit_input(&SessionInput {
            id: "steer-1".into(),
            session_id: "skill-mid-run".into(),
            delivery: Delivery::Steer,
            prompt: "STEER-MARKER".into(),
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    // Clone the Arc so the event handler can update the skill mid-run.
    let skill_handle = s.skill_prompt.clone();
    let skill_set = Arc::new(AtomicBool::new(false));
    let skill_set_clone = skill_set.clone();

    // Spawn the run in a separate task so we can update the skill concurrently.
    // The event handler sets the skill when it sees ToolStart during turn 1's
    // bash execution — deterministic, before the turn boundary where the steer
    // is promoted and turn 2 begins.
    let run_task = tokio::spawn(async move {
        run(&mut s, "kickoff".into(), move |ev| {
            if matches!(ev, SessionEvent::ToolStart { .. })
                && !skill_set_clone.load(Ordering::SeqCst)
            {
                *skill_handle.lock().unwrap() = Some("MID-RUN-SKILL".into());
                skill_set_clone.store(true, Ordering::SeqCst);
            }
        })
        .await
    });

    run_task.await.unwrap().unwrap();

    // The skill must have been set during turn 1.
    assert!(
        skill_set.load(Ordering::SeqCst),
        "skill should have been set during turn 1's tool execution"
    );

    let requests = mock.requests();
    assert!(
        requests.len() >= 2,
        "expected at least 2 LLM calls, got {}",
        requests.len()
    );

    // Turn 1's system prompt must NOT contain the skill (it was set during
    // tool execution, after the request was already sent).
    let first_system = system_content(&requests[0]);
    assert!(
        !first_system.contains("MID-RUN-SKILL"),
        "turn 1 system prompt must NOT contain the skill (not yet set): {first_system}"
    );

    // Turn 2's system prompt MUST contain the skill.
    let second_system = system_content(&requests[1]);
    assert!(
        second_system.contains("MID-RUN-SKILL"),
        "turn 2 system prompt must contain the mid-run skill: {second_system}"
    );
}

/// Same scenario but with a queued follow-up instead of a steer. The queue
/// is consumed at the idle boundary after turn 2 (which has no tool calls),
/// so a third turn is needed.
///
/// Flow:
/// 1. Turn 1: bash → ToolStart → skill set via Arc
/// 2. Turn 2: done → idle → consume queue → continue
/// 3. Turn 3: done → idle → no queue → Done
#[tokio::test]
async fn skill_set_mid_run_appears_in_queue_followup_turn() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn(1)])
            .push_script(vec![done_turn("d1")])
            .push_script(vec![done_turn("d2")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "skill-queue",
        agent,
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    store
        .create_session(&opencoder_store::SessionMeta {
            id: "skill-queue".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();

    // Admit a queue follow-up BEFORE the run.
    store
        .admit_input(&SessionInput {
            id: "q-1".into(),
            session_id: "skill-queue".into(),
            delivery: Delivery::Queue,
            prompt: "follow-up".into(),
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    let skill_handle = s.skill_prompt.clone();
    let skill_set = Arc::new(AtomicBool::new(false));
    let skill_set_clone = skill_set.clone();

    let run_task = tokio::spawn(async move {
        run(&mut s, "kickoff".into(), move |ev| {
            if matches!(ev, SessionEvent::ToolStart { .. })
                && !skill_set_clone.load(Ordering::SeqCst)
            {
                *skill_handle.lock().unwrap() = Some("QUEUE-SKILL".into());
                skill_set_clone.store(true, Ordering::SeqCst);
            }
        })
        .await
    });

    run_task.await.unwrap().unwrap();

    assert!(
        skill_set.load(Ordering::SeqCst),
        "skill should have been set during turn 1"
    );

    let requests = mock.requests();
    assert!(
        requests.len() >= 3,
        "expected at least 3 LLM calls (bash + done + queue follow-up), got {}",
        requests.len()
    );

    // Turn 1: no skill (set during tool execution, after request sent).
    assert!(
        !system_content(&requests[0]).contains("QUEUE-SKILL"),
        "turn 1 must not have skill"
    );

    // Turn 3 (queue follow-up): must have the skill.
    let third_system = system_content(&requests[2]);
    assert!(
        third_system.contains("QUEUE-SKILL"),
        "turn 3 (queue follow-up) system prompt must contain the skill: {third_system}"
    );
}

/// `set_skill` and `skill_prompt_cloned` round-trip on a fresh session.
#[tokio::test]
async fn set_skill_and_clone_roundtrip() {
    let mock =
        Arc::new(MockChatClient::new().with_default(vec![done_turn("ok")])) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new("rt", agent, config(), mock, dir.path().to_path_buf());

    assert!(s.skill_prompt_cloned().is_none());
    s.set_skill(Some("hello".into()));
    assert_eq!(s.skill_prompt_cloned().as_deref(), Some("hello"));
    s.set_skill(None);
    assert!(s.skill_prompt_cloned().is_none());
}

/// `with_skill` builder still works and is visible via `skill_prompt_cloned`.
#[tokio::test]
async fn with_skill_builder_sets_skill() {
    let mock =
        Arc::new(MockChatClient::new().with_default(vec![done_turn("ok")])) as Arc<dyn ChatStream>;
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let s = SessionState::new("ws", agent, config(), mock, dir.path().to_path_buf())
        .with_skill("BUILDER-SKILL".into());
    assert_eq!(s.skill_prompt_cloned().as_deref(), Some("BUILDER-SKILL"));
}
