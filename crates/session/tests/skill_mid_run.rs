//! Skill activation mid-run: when a skill is set via the shared `Arc<Mutex>`
//! while a session is running (between turns), the next turn's request must
//! carry the skill — as a transient `[active skill]` tail reminder naming the
//! skill's source file, NOT as system-prompt content (skill bodies never ship
//! in the system prompt; see `opencoder_session::skill_context`).
//!
//! Before the fix, `skill_prompt` was `Option<String>` updated through the
//! cmd channel (`UiCmd::SetSkill`). While `run_loop` was executing, the
//! worker could not process cmd-channel messages until `run_loop` returned,
//! so the skill never reached the turn that needed it. The fix makes
//! `skill_prompt` an `Arc<Mutex<Option<String>>>` so the TUI can update it
//! directly, and `run_one_llm_call` reads the latest value each turn.
//!
//! Skill bodies are stored with the `> Source: <path>` prefix that
//! `opencoder_core::body_with_source` writes (mirroring the TUI `$` picker /
//! `skill_resolve` storage), which is what lets the tail reminder surface
//! the skill's path.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, run_with_images, SessionEvent, SessionState};
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
            ..Default::default()
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

/// Extract the content of the LAST user-role message of a ChatRequest —
/// where the transient `[active skill]` tail reminder is appended.
fn last_user_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or("")
        .to_string()
}

/// Whether any user message of the request carries the `[active skill]`
/// tail reminder.
fn has_active_skill_reminder(req: &opencoder_llm::ChatRequest) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("[active skill]"))
    })
}

/// The skill body ships as the one-shot `[skill loaded]` payload
/// message naming `path` (the `[active skill]` tail pointer is
/// fallback-only).
fn has_loaded_skill_message(req: &opencoder_llm::ChatRequest, path: &str) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.starts_with("[skill loaded] ") && c.contains(path))
    })
}

/// Skill body as the TUI `$` picker / `skill_resolve` actually store it:
/// the `> Source:` prefix (`opencoder_core::body_with_source` format) that
/// the tail reminder parses the skill's path from.
fn sourced_body(path: &str, body: &str) -> String {
    format!("> Source: {path}\n\n{body}")
}

/// Create the session row so input admission (FK) succeeds before the run.
async fn seed_session(store: &Arc<dyn Store>) {
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "skill-mid-run".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
}

/// When a skill is set via the shared `Arc<Mutex>` during turn 1's tool
/// execution, turn 2's request must carry the skill as a transient tail
/// reminder (last user message, `[active skill]` + source path) — even
/// though turn 1's request carried nothing.
///
/// Flow:
/// 1. Turn 1: bash tool call → ToolStart event fires → skill is set via Arc
/// 2. Turn boundary: a pre-admitted steer is promoted into history
/// 3. Turn 2: `run_one_llm_call` reads `skill_prompt_cloned()` → finds the skill
/// 4. Turn 2: done (no tool calls) → idle → Done
#[tokio::test]
async fn skill_set_mid_run_appears_in_next_turn_tail_reminder() {
    let store = mem_store().await;
    let mock: Arc<MockChatClient> = Arc::new(
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
            seq: None,
            id: "steer-1".into(),
            session_id: "skill-mid-run".into(),
            delivery: Delivery::Steer,
            prompt: "STEER-MARKER".into(),
            images: Vec::new(),
            display_text: None,
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
                *skill_handle.lock().unwrap() =
                    Some(sourced_body("/skills/mid-run/SKILL.md", "MID-RUN-SKILL"));
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

    // Turn 1's request must carry no trace of the skill (it was set during
    // tool execution, after the request was already sent): no body in the
    // system prompt and no tail-reminder message at all.
    let first_system = system_content(&requests[0]);
    assert!(
        !first_system.contains("MID-RUN-SKILL"),
        "turn 1 system prompt must NOT contain the skill (not yet set): {first_system}"
    );
    assert!(
        !has_active_skill_reminder(&requests[0]),
        "turn 1 payload must carry no [active skill] reminder: {:?}",
        requests[0].messages
    );

    // Turn 2's system prompt still excludes the body; the skill arrives as
    // the one-shot `[skill loaded]` payload message (the
    // `[active skill]` tail pointer is fallback-only and stays silent).
    let second_system = system_content(&requests[1]);
    assert!(
        !second_system.contains("MID-RUN-SKILL"),
        "skill bodies never ship in the system prompt: {second_system}"
    );
    let tail = last_user_content(&requests[1]);
    assert!(
        !tail.contains("[active skill]"),
        "pointer suppressed while the loaded marker is present: {tail}"
    );
    assert!(
        has_loaded_skill_message(&requests[1], "/skills/mid-run/SKILL.md"),
        "turn 2 receives the mid-run skill via the [skill loaded] message"
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
async fn skill_set_mid_run_delivers_once_before_queue_followup() {
    let store = mem_store().await;
    let mock: Arc<MockChatClient> = Arc::new(
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

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // Admit a queue follow-up BEFORE the run.
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "q-1".into(),
            session_id: "skill-queue".into(),
            delivery: Delivery::Queue,
            prompt: "follow-up".into(),
            images: Vec::new(),
            display_text: None,
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
                *skill_handle.lock().unwrap() =
                    Some(sourced_body("/skills/queue/SKILL.md", "QUEUE-SKILL"));
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
    assert!(
        !has_active_skill_reminder(&requests[0]),
        "turn 1 payload must carry no [active skill] reminder"
    );

    // The skill set mid-run arrives via the ONE-SHOT `[skill loaded]`
    // payload message on the FIRST round that observes it (turn 2, the
    // post-bash round) — and ONLY that round: the queue follow-up (turn 3)
    // carries no skill body at all, the delivered marker being the model's
    // pointer back to the source file. The system prompt stays skill-free
    // and the tail pointer stays silent throughout.
    let second_system = system_content(&requests[1]);
    assert!(
        !second_system.contains("QUEUE-SKILL"),
        "skill bodies never ship in the system prompt: {second_system}"
    );
    let second_tail = last_user_content(&requests[1]);
    assert!(
        !second_tail.contains("[active skill]"),
        "pointer suppressed while the loaded marker is present: {second_tail}"
    );
    assert!(
        has_loaded_skill_message(&requests[1], "/skills/queue/SKILL.md"),
        "turn 2 (first round observing the mid-run skill) delivers the body once"
    );
    assert!(
        !has_loaded_skill_message(&requests[2], "/skills/queue/SKILL.md"),
        "turn 3 (queue follow-up) carries NO skill body — one-shot delivery spent"
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

/// Skill-only submit: when the prompt is empty but a skill is active, the
/// runner must still execute a turn (drain mode) so the model reads the
/// skill (via its `[active skill]` tail reminder) and acts on it.
///
/// Flow:
/// 1. Skill is set on the session via `set_skill` (mirrors TUI
///    `apply_skill_tokens` writing to the shared `Arc<Mutex>`).
/// 2. `run` is called with an empty prompt — a synthetic trigger user
///    message is injected so the model records a user turn.
/// 3. `run_one_llm_call` reads `skill_prompt_cloned()` → finds the skill.
/// 4. Turn: done (no tool calls) → idle → no queue → Done.
#[tokio::test]
async fn skill_only_empty_prompt_starts_turn_with_skill_tail_reminder() {
    let store = mem_store().await;
    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("skill executed")]));
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "skill-only-submit",
        agent,
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    store
        .create_session(&opencoder_store::SessionMeta {
            id: "skill-only-submit".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // Set the skill before the run (Source-prefixed body, as the TUI /
    // skill_resolve store it).
    s.set_skill(Some(sourced_body(
        "/skills/do-the-thing/SKILL.md",
        "DO-THE-THING",
    )));

    // Empty prompt with an active skill: a synthetic trigger user message is
    // injected so the model records a user turn and acts on the skill body.
    run(&mut s, String::new(), |_| {}).await.unwrap();

    let requests = mock.requests();
    assert_eq!(
        requests.len(),
        1,
        "empty-prompt skill submit must trigger exactly one LLM call, got {}",
        requests.len()
    );

    let system = system_content(&requests[0]);
    assert!(
        !system.contains("DO-THE-THING"),
        "skill bodies never ship in the system prompt: {system}"
    );
    let tail = last_user_content(&requests[0]);
    assert!(
        !tail.contains("[active skill]"),
        "pointer suppressed while the loaded marker is present: {tail}"
    );
    assert!(
        has_loaded_skill_message(&requests[0], "/skills/do-the-thing/SKILL.md"),
        "the skill-only submit delivers the body via the [skill loaded] message"
    );

    // A synthetic trigger user message must be recorded for skill-only submits
    // so the model acts on the skill body rather than seeing it passively.
    assert!(
        s.messages.iter().any(|m| {
            m.role == Role::User && m.text().contains("active skill is now in effect")
        }),
        "expected a user-role trigger message after skill-only submit"
    );
}

/// Skill-only submit records the synthetic trigger as the final user turn.
///
/// After a skill-only submit, the last recorded message must be the synthetic
/// trigger (user-role, `synthetic == true`) — verifying the model acts on it
/// as the most recent recorded turn, rather than only seeing the skill
/// passively via its tail reminder.
#[tokio::test]
async fn skill_only_empty_prompt_records_user_trigger_message() {
    let store = mem_store().await;
    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("skill executed")]));
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "skill-only-trigger",
        agent,
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    store
        .create_session(&opencoder_store::SessionMeta {
            id: "skill-only-trigger".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // Set the skill before the run.
    s.set_skill(Some("MY-SKILL-BODY".into()));

    // Empty prompt with an active skill: a synthetic trigger user message is
    // injected so the model records a user turn.
    run(&mut s, String::new(), |_| {}).await.unwrap();

    // The recorded transcript is [user trigger (synthetic), assistant
    // response]: the last *user*-role message must be the synthetic trigger
    // — i.e. the trigger is the most recent user turn the model acted on.
    let last_user = s
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .expect("expected a recorded user message");
    assert!(
        last_user.synthetic,
        "the last user message must be the synthetic trigger"
    );
    assert!(
        last_user.text().contains("active skill is now in effect"),
        "the trigger text must reference the active skill: {}",
        last_user.text()
    );

    // The model must have acted on the trigger: the final recorded message is
    // an assistant response following the trigger.
    assert_eq!(
        s.messages.last().unwrap().role,
        Role::Assistant,
        "the final message must be the assistant response acting on the trigger"
    );
}

#[tokio::test]
async fn image_only_turn_with_skill_records_both_user_image_and_trigger() {
    // Gap 3: when an image-only turn (empty text + images) is submitted with
    // an active skill, both the user image message AND the synthetic skill
    // trigger must be recorded — they are no longer mutually exclusive.
    let store = mem_store().await;
    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("skill on image")]));
    let client: Arc<dyn ChatStream> = mock.clone();

    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "image-skill-trigger",
        agent,
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    store
        .create_session(&opencoder_store::SessionMeta {
            id: "image-skill-trigger".into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    s.set_skill(Some("MY-SKILL-BODY".into()));

    // Image-only turn (empty text + one image) with an active skill.
    run_with_images(
        &mut s,
        String::new(),
        vec!["data:image/png;base64,AAAA".into()],
        |_| {},
    )
    .await
    .unwrap();

    // The transcript should contain:
    //   1. A user message with the image (non-synthetic).
    //   2. A synthetic user trigger message ("active skill is now in effect").
    //   3. An assistant response.
    let user_msgs: Vec<_> = s.messages.iter().filter(|m| m.role == Role::User).collect();
    assert!(
        user_msgs.len() >= 2,
        "expected at least 2 user messages (image + trigger), got {}",
        user_msgs.len()
    );

    // The non-synthetic user message must carry the image.
    let image_msg = user_msgs
        .iter()
        .find(|m| !m.synthetic)
        .expect("expected a non-synthetic user message with the image");
    assert!(
        image_msg
            .blocks
            .iter()
            .any(|b| matches!(b, opencoder_core::ContentBlock::Image { .. })),
        "the user message must contain an Image block"
    );

    // The synthetic trigger must also be present.
    let trigger_msg = user_msgs
        .iter()
        .find(|m| m.synthetic)
        .expect("expected a synthetic skill trigger message");
    assert!(
        trigger_msg.text().contains("active skill is now in effect"),
        "trigger must reference the active skill: {}",
        trigger_msg.text()
    );
}
