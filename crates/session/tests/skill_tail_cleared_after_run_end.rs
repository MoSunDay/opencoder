//! Decisive reproduction: a PRE-SET skill — the state left by a web
//! `SetSkill`-before-run (`admit_and_drain_guarded` persists `sessions.skill`
//! before admitting the prompt) or by a crash MID-run (row still set when a
//! later run resumes) — must be wiped when that run ends.
//!
//! Contract under test (`skill_lifecycle::run_loop_one_shot` →
//! `clear_on_run_end`): when a run ends the skill must be gone from
//!   (a) memory: `SessionState::skill_prompt_cloned()` is `None`,
//!   (b) store: the session row's `skill` column is NULL (defeats resume
//!       resurrection),
//!   (c) payload: every LLM request of any FOLLOWING run carries NO
//!       `[active skill]` tail reminder — the synthetic trailing user message
//!       `skill_context::tail_reminder` derives per call and
//!       `runner/llm_call.rs` appends last to the payload.
//!
//! No src code is touched by this file: it only pins observed behavior.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencoder_core::{Config, ToolArc};
use opencoder_llm::{ChatStream, ChatRequest, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::runner::run_with_registry;
use opencoder_session::{resume, run, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, SessionPatch, Store};

/// `body_with_source`-style skill body: leading `> Source:` path line + body
/// text — exactly the shape `tail_reminder` parses the active path from.
const SKILL_BODY: &str = "> Source: /skills/x/SKILL.md\n\nX-BODY: follow the skill.";

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
        usage: Some(Usage::default()),
    }
}

/// A tool-call turn so the loop keeps going past one LLM round.
fn bash_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![CompletedToolCall {
            id: "tu1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "true"}),
        }],
        usage: Some(Usage::default()),
    }
}

/// True when any user-role message in the lowered payload carries the
/// `[active skill]` tail reminder (the assertion style of
/// `tests/steer_skill_deferral.rs` / `tests/skill_one_shot.rs`).
fn has_active_skill_tail(req: &ChatRequest) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("[active skill]"))
    })
}

/// The armed skill ships as the in-conversation `[skill loaded]` message
/// (the `[active skill]` tail pointer is fallback-only under F3).
fn has_loaded_skill_message(req: &ChatRequest) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.starts_with("[skill loaded] "))
    })
}

fn user_texts(req: &ChatRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(str::to_string)
        .collect()
}

/// Build the pre-set-skill session: the row carries `sessions.skill` (what
/// web `SetSkill`-before-run / a mid-run crash leaves behind) and `resume`
/// restores it into memory — the exact entry state under test. Returns the
/// session, the store, and the working dir (kept alive for the run).
async fn preset_skill_session(
    id: &str,
    client: Arc<dyn ChatStream>,
) -> anyhow::Result<(SessionState, Arc<dyn Store>, tempfile::TempDir)> {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    // Crash-mid-run shape: some prior history, skill still set on the row.
    store
        .append_message(id, &opencoder_core::Message::user("u0", "earlier turn"))
        .await
        .unwrap();
    store
        .update_session(
            id,
            &SessionPatch {
                skill: Some(SKILL_BODY.into()),
                updated_at: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let s = resume(store.clone(), id, config(), client, dir.path().to_path_buf()).await?;
    assert_eq!(
        s.skill_prompt_cloned().as_deref(),
        Some(SKILL_BODY),
        "fixture: preset skill restored into memory by resume"
    );
    Ok((s, store, dir))
}

/// Memory + store cleared, shared by every variant.
async fn assert_cleared(s: &SessionState, store: &Arc<dyn Store>, id: &str, ctx: &str) {
    assert!(
        s.skill_prompt_cloned().is_none(),
        "{ctx}: memory skill_prompt must be None after the run ends"
    );
    let meta = store
        .get_session(id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{ctx}: session row must exist"));
    assert!(
        meta.skill.is_none(),
        "{ctx}: store row `skill` must be NULL after the run ends, got {:?}",
        meta.skill
    );
}

// ---------------------------------------------------------------------------
// 1. Plain run: preset skill -> Done -> second run carries no tail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn preset_skill_tail_cleared_after_done_run() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![done_turn("skill work")])
            .push_script(vec![done_turn("plain work")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (mut s, store, _dir) = preset_skill_session("tail-clear-done", client).await
        .unwrap_or_else(|e| panic!("fixture: {e}"));

    // Run 1 on the pre-set skill, driven to Done.
    run(&mut s, "do the thing".into(), |_| {}).await.unwrap();

    // Sanity: run 1's payload DID carry the skill body (the preset was armed).
    let first = &mock.requests()[0];
    assert!(
        has_loaded_skill_message(first),
        "run 1 request must carry the [skill loaded] body while preset: {:?}",
        user_texts(first)
    );

    // 断言一 (memory) + 断言二 (store row NULL).
    assert_cleared(&s, &store, "tail-clear-done", "after run 1 (Done)").await;

    // Run 2: a plain prompt — its payload must carry NO tail anywhere.
    run(&mut s, "plain follow up".into(), |_| {})
        .await
        .unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "two runs -> exactly two LLM calls");
    // 断言三: no message in run 2's payload contains "[active skill]".
    assert!(
        !has_active_skill_tail(&reqs[1]),
        "run 2 request still carries the [active skill] tail: {:?}",
        user_texts(&reqs[1])
    );
}

// ---------------------------------------------------------------------------
// 2. Tool-call multi-turn run: preset skill -> bash turn -> Done -> clean
// ---------------------------------------------------------------------------

#[tokio::test]
async fn preset_skill_tail_cleared_after_tool_call_run() {
    // One push_script per LLM round: round 1 calls the tool, round 2 done.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("use the tool")])
            .push_script(vec![done_turn("skill work")])
            .push_script(vec![done_turn("plain work")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (mut s, store, _dir) = preset_skill_session("tail-clear-tools", client).await
        .unwrap_or_else(|e| panic!("fixture: {e}"));

    // Run 1: two LLM rounds (tool call, then Done).
    run(&mut s, "do the thing".into(), |_| {}).await.unwrap();

    let reqs_after_run1 = mock.requests();
    assert_eq!(
        reqs_after_run1.len(),
        2,
        "run 1 spans a tool round: two requests"
    );
    assert!(
        reqs_after_run1.iter().all(has_loaded_skill_message),
        "every run-1 round carries the loaded skill body while the skill is active"
    );

    assert_cleared(&s, &store, "tail-clear-tools", "after run 1 (tool run)").await;

    run(&mut s, "plain follow up".into(), |_| {})
        .await
        .unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 3, "run 2 adds exactly one request");
    assert!(
        !has_active_skill_tail(&reqs[2]),
        "post-tool-run request still carries the [active skill] tail: {:?}",
        user_texts(&reqs[2])
    );
}

// ---------------------------------------------------------------------------
// 3. Steer carries `/act_clear_context` mid-run: run ends, next run clean
// ---------------------------------------------------------------------------

/// Parks a turn inside tool execution so the test can admit a steer while
/// the run is in flight (same technique as `tests/steer_skill_deferral.rs`).
struct GateTool(&'static str, tokio::sync::watch::Receiver<bool>);

#[async_trait::async_trait]
impl opencoder_core::Tool for GateTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "blocks until opened"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &opencoder_core::ToolContext,
    ) -> anyhow::Result<opencoder_core::ToolOutput> {
        let mut rx = self.1.clone();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
        Ok(opencoder_core::ToolOutput::ok("gate opened"))
    }
}

#[tokio::test]
async fn steer_clear_context_mid_run_leaves_next_run_tail_free() {
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_turn("turn-1")])
            .push_script(vec![done_turn("continuity work")])
            .push_script(vec![done_turn("plain follow-up")]),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let (mut s, store, _dir) = preset_skill_session("tail-clear-steer", client).await
        .unwrap_or_else(|e| panic!("fixture: {e}"));

    let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);
    let mut registry: HashMap<String, ToolArc> = HashMap::new();
    registry.insert("gate".into(), Arc::new(GateTool("gate", gate_rx)) as ToolArc);

    let started = Arc::new(AtomicBool::new(false));
    let started_flag = started.clone();
    let run_task = tokio::spawn(async move {
        let res = run_with_registry(
            &mut s,
            "kickoff".into(),
            Vec::new(),
            &registry,
            move |ev| {
                if matches!(ev, SessionEvent::ToolStart { .. }) {
                    started_flag.store(true, Ordering::SeqCst);
                }
            },
        )
        .await;
        (s, res)
    });

    // Park until turn 1 is inside the gate tool (request #1 already sent,
    // tail armed by the preset).
    while !started.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        has_loaded_skill_message(&mock.requests()[0]),
        "run 1 turn 1 must carry the loaded skill body while the preset is active"
    );

    // Mid-run: admit the control command as a steer.
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "steer-cc".into(),
            session_id: "tail-clear-steer".into(),
            delivery: Delivery::Steer,
            prompt: "/act_clear_context".into(),
            images: vec![],
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();

    // Release the gate; the turn boundary absorbs the steer and runs the
    // preserved continuity seed once, matching the idle/queue ingress paths.
    gate_tx.send(true).unwrap();
    let (s, res) = run_task.await.unwrap();
    res.unwrap();

    assert_cleared(&s, &store, "tail-clear-steer", "after steer run").await;
    assert!(
        !has_active_skill_tail(&mock.requests()[1]),
        "clear-context continuation must not inherit the cleared skill tail"
    );

    // Second run: no tail in its payload.
    let mut s = s;
    run(&mut s, "plain follow up".into(), |_| {})
        .await
        .unwrap();
    let reqs = mock.requests();
    let last = reqs.last().unwrap();
    assert!(
        !has_active_skill_tail(last),
        "run 2 request after /act_clear_context steer still carries the tail: {:?}",
        user_texts(last)
    );
}
