//! Steer-path skill deferral (P0 regression): a `$skill` steer admitted
//! MID-turn must not arm the skill for the already-sent request of that
//! turn. Activation happens exactly at steer absorption — the next turn
//! boundary (`record_compound` in the drain loop) — so only the FOLLOWING
//! request carries the `[active skill]` tail reminder, and only then is
//! `sessions.skill` persisted.
//!
//! The admission surfaces (TUI Enter-while-running, web `POST /prompt`)
//! store the raw `$name` text verbatim; the runner owns the timing. These
//! tests pin that runner contract.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_session::runner::run_with_registry;
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};

/// Serializes tests that mutate process-global HOME (`seed_builtin_skills`
/// discovers skills under `$HOME/.opencoder/skills`).
static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII: points HOME + XDG_CONFIG_HOME at `home` while held.
struct HomeGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

fn lock_home(home: &std::path::Path) -> HomeGuard {
    let _lock = HOME_MUTEX.lock().unwrap();
    let prev_home = std::env::var_os("HOME");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("HOME", home);
    std::env::set_var("XDG_CONFIG_HOME", home);
    HomeGuard {
        prev_home,
        prev_xdg,
        _lock,
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match self.prev_xdg.take() {
            Some(h) => std::env::set_var("XDG_CONFIG_HOME", h),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

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

/// Turn 1's scripted event: call the gate tool so the loop continues and the
/// turn stays parked inside tool execution.
fn gate_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "turn-1".into(),
        tool_calls: vec![CompletedToolCall {
            id: "tu1".into(),
            name: "gate".into(),
            input: serde_json::json!({}),
        }],
        usage: None,
    }
}

/// Turn 2's scripted event: call the second gate tool so the run parks AGAIN
/// — now PAST the steer absorption boundary, letting the test observe the
/// consumption-time persistence while the run is still in flight.
fn gate2_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "turn-2".into(),
        tool_calls: vec![CompletedToolCall {
            id: "tu2".into(),
            name: "gate2".into(),
            input: serde_json::json!({}),
        }],
        usage: None,
    }
}

fn has_active_skill_reminder(req: &opencoder_llm::ChatRequest) -> bool {
    req.messages.iter().any(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && m.get("content")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("[active skill]"))
    })
}

fn last_user_content(req: &opencoder_llm::ChatRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .unwrap_or_default()
        .to_string()
}

/// Blocks inside its `execute` until the test opens the gate, parking a turn
/// mid-execution: the request is already sent, the next absorption boundary
/// has not been reached. `name` lets one impl serve several gate slots.
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

/// A `$skill` steer admitted MID-turn (turn 1 parked inside the gate tool)
/// must NOT arm the skill for the already-sent request #1: activation happens
/// exactly at steer absorption (the next turn boundary → `record_compound`),
/// so only request #2 carries the `[active skill]` tail reminder and only
/// then does `sessions.skill` get persisted.
#[tokio::test]
async fn steer_admitted_mid_turn_defers_skill_until_absorption() {
    let home = tempfile::tempdir().unwrap();
    let _guard = lock_home(home.path());
    opencoder_core::seed_builtin_skills();

    let store = mem_store().await;
    let sid = "steer-mid-run-skill";
    store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.into(),
            title: Some("t".into()),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();

    let mock: Arc<MockChatClient> = Arc::new(
        MockChatClient::new()
            .push_script(vec![gate_turn()])
            .push_script(vec![gate2_turn()])
            .push_script(vec![done_turn("final reply")]),
    );

    let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);
    let (gate2_tx, gate2_rx) = tokio::sync::watch::channel(false);
    let mut registry = std::collections::HashMap::new();
    registry.insert(
        "gate".to_string(),
        Arc::new(GateTool("gate", gate_rx)) as opencoder_core::ToolArc,
    );
    registry.insert(
        "gate2".to_string(),
        Arc::new(GateTool("gate2", gate2_rx)) as opencoder_core::ToolArc,
    );

    let tool_started = Arc::new(AtomicBool::new(false));
    let started_flag = tool_started.clone();

    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        sid,
        resolve_agent("act").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());

    let run_task = tokio::spawn(async move {
        run_with_registry(
            &mut session,
            "kickoff".into(),
            Vec::new(),
            &registry,
            move |ev| {
                if matches!(ev, SessionEvent::ToolStart { .. }) {
                    started_flag.store(true, Ordering::SeqCst);
                }
            },
        )
        .await
    });

    // Park until turn 1 is executing the gate tool (request #1 already sent).
    while !tool_started.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // Mid-turn: request #1 carries no skill, and nothing was persisted yet.
    let req1 = &mock.requests()[0];
    assert!(
        !has_active_skill_reminder(req1),
        "turn 1 payload must carry no [active skill] reminder: {:?}",
        req1.messages
    );
    let mid = store.get_session(sid).await.unwrap().unwrap();
    assert!(mid.skill.is_none(), "skill not persisted before absorption");

    // Admit the steer mid-turn; the pending row keeps the raw token.
    store
        .admit_input(&SessionInput {
            seq: None,
            id: "steer-1".into(),
            session_id: sid.into(),
            delivery: Delivery::Steer,
            prompt: "$review analyze this".into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();
    let pending = store.pending_inputs(sid, Delivery::Steer).await.unwrap();
    assert!(
        pending
            .iter()
            .any(|i| i.delivery == Delivery::Steer && i.prompt.contains("$review")),
        "admitted steer row keeps the raw token: {pending:?}"
    );

    // Release the gate: the boundary absorbs the steer and turn 2 runs.
    gate_tx.send(true).unwrap();

    // Park again inside gate2 (request #2 already sent = the absorption
    // boundary is behind us: the $review token was resolved at it).
    while mock.requests().len() < 2 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let requests = mock.requests();

    // Turn 1 (already sent before the steer existed) stays untouched.
    assert!(!has_active_skill_reminder(&requests[0]));
    let req1_text = requests[0]
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !req1_text.contains("$review"),
        "turn 1 payload never sees the token: {req1_text}"
    );

    // Turn 2 carries the skill via the one-shot `[skill loaded]`
    // payload message (the tail pointer stays silent), token stripped.
    let tail = last_user_content(&requests[1]);
    assert!(
        !tail.contains("[active skill]"),
        "pointer silent while the body ships adjacent: {tail}"
    );
    assert!(
        requests[1].messages.iter().any(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("user")
                && m.get("content")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| {
                        c.starts_with("[skill loaded] ")
                            && c.contains("skills/review/SKILL.md")
                    })
        }),
        "turn 2 receives the skill via the [skill loaded] message naming its source"
    );
    let user_texts: Vec<String> = requests[1]
        .messages
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .map(str::to_string)
        .collect();
    assert!(
        user_texts.iter().any(|t| t.contains("analyze this")),
        "steer text runs as a real prompt: {user_texts:?}"
    );
    assert!(
        !user_texts.iter().any(|t| t.contains("$review")),
        "$review token stripped from every user message: {user_texts:?}"
    );

    // Consumption-time persistence, observed WHILE the run is still parked:
    // the absorption boundary resolved the token and wrote the body to
    // `sessions.skill` (this is the write a mid-run crash + resume replays).
    let absorbed = store.get_session(sid).await.unwrap().unwrap();
    let skill = absorbed.skill.expect("skill persisted at absorption");
    assert!(
        skill.contains("> Source: "),
        "persisted body keeps the source prefix: {skill}"
    );

    // Release gate2: the run completes and the ONE-SHOT run-end clear lands —
    // memory and store both lose the skill again.
    gate2_tx.send(true).unwrap();
    run_task.await.unwrap().unwrap();

    assert_eq!(mock.requests().len(), 3, "exactly three LLM calls");
    let after = store.get_session(sid).await.unwrap().unwrap();
    assert!(
        after.skill.is_none(),
        "one-shot: the completed run clears the persisted skill: {:?}",
        after.skill
    );
}
