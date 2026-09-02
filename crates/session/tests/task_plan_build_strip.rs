//! While the task-plan skill is active (the condition that turns the TUI
//! `act` chip yellow), NO prompt surface may advertise the 'build'
//! (implementation) subagent — the same hiding sandbox mode applies
//! unconditionally. Asserted on the REAL outbound `ChatRequest` (system
//! message + `task` tool schema) via the recording mock, plus the runner's
//! unknown-subagent_type error text.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::SessionState;

/// The clause every hidden surface must lose.
const BUILD_CLAUSE: &str = "'build' (full tools)";

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "ok".into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

/// A body with the exact `> Source:` line `body_with_source` writes for the
/// seeded task-plan skill — the real activation shape.
fn plan_body() -> String {
    "> Source: /home/u/.opencoder/skills/task-plan/SKILL.md\n\nplan body".into()
}

fn session_for(
    id: &str,
    agent: &str,
    skill_body: Option<String>,
) -> (SessionState, Arc<MockChatClient>) {
    let mock = Arc::new(MockChatClient::new().push_script(vec![done_turn()]));
    let dir = tempfile::tempdir().unwrap();
    let session = SessionState::new(
        id,
        resolve_agent(agent).unwrap(),
        config(),
        mock.clone(),
        dir.path().to_path_buf(),
    );
    if let Some(body) = skill_body {
        session.set_skill(Some(body));
    }
    (session, mock)
}

/// Run one turn and return the single captured request.
async fn single_request(
    session: SessionState,
    mock: Arc<MockChatClient>,
) -> opencoder_llm::ChatRequest {
    let mut inner = session;
    let mut events = Vec::new();
    opencoder_session::run(&mut inner, "do the thing".into(), |ev| events.push(ev))
        .await
        .unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1, "exactly one LLM round");
    reqs.into_iter().next().unwrap()
}

fn task_schema_text(req: &opencoder_llm::ChatRequest) -> String {
    req.tools
        .iter()
        .find(|t| t["function"]["name"] == "task")
        .expect("task tool advertised to a primary agent")["function"]
        .to_string()
}

#[tokio::test]
async fn task_plan_act_request_hides_build_on_every_surface() {
    let (session, mock) = session_for("plan-strip-act", "act", Some(plan_body()));
    let req = single_request(session, mock).await;

    let system = req
        .messages
        .iter()
        .find(|m| m["role"] == "system")
        .expect("system message present")["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !system.contains(BUILD_CLAUSE),
        "task-plan act system prompt must not advertise build, got: {system}"
    );
    assert!(
        system.contains("'explore' (read-only)"),
        "the explore advertisement must survive, got: {system}"
    );

    let task = task_schema_text(&req);
    assert!(
        !task.contains("build"),
        "task-plan act task schema must not mention build, got: {task}"
    );
}

#[tokio::test]
async fn plain_act_request_still_advertises_build() {
    // Control: without the skill, act keeps the full delegation line and
    // the build clause in the task schema.
    let (session, mock) = session_for("plan-strip-plain", "act", None);
    let req = single_request(session, mock).await;
    let system = req.messages[0]["content"].as_str().unwrap().to_string();
    assert!(
        system.contains(BUILD_CLAUSE),
        "plain act system prompt must keep the build clause, got: {system}"
    );
    assert!(
        task_schema_text(&req).contains("build"),
        "plain act task schema must still advertise build"
    );
}

#[tokio::test]
async fn task_plan_unknown_subagent_type_error_omits_build() {
    // Tool results are prompt content too: the valid-options error must not
    // introduce 'build' to a model that was never told it exists.
    let dir = tempfile::tempdir().unwrap();
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "delegating".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "task-1".into(),
                    name: "task".into(),
                    input: serde_json::json!({"prompt": "look", "subagent_type": "ninja"}),
                }],
                usage: Some(Usage::default()),
            }])
            .push_script(vec![done_turn()]),
    );
    let session = SessionState::new(
        "plan-strip-ninja",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        dir.path().to_path_buf(),
    );
    session.set_skill(Some(plan_body()));
    let mut inner = session;
    let mut events = Vec::new();
    opencoder_session::run(&mut inner, "do the thing".into(), |ev| events.push(ev))
        .await
        .unwrap();
    let err = events
        .iter()
        .find_map(|ev| match ev {
            opencoder_session::SessionEvent::ToolEnd {
                name,
                output,
                is_error,
                ..
            } if *is_error && name == "task" => Some(output.clone()),
            _ => None,
        })
        .expect("unknown subagent_type must error");
    assert!(
        err.contains("Unknown subagent_type 'ninja'"),
        "expected the unknown-type error, got: {err}"
    );
    assert!(
        !err.contains("build"),
        "task-plan error text must not advertise build, got: {err}"
    );
}

/// THE reported bug, end to end: a task-plan run that dies abnormally (LLM
/// error / Esc hard-cancel) must KEEP the skill (memory + store row), so the
/// continued/resumed task still plans skill-armed (no build advertisement);
/// only a run that actually COMPLETES clears it — afterwards the build
/// subagent is advertised again (post-compaction system prompt included,
/// since that prompt is rebuilt per call).
#[tokio::test]
async fn aborted_run_keeps_task_plan_completed_run_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let mut session = SessionState::new(
        "plan-strip-lifecycle",
        resolve_agent("act").unwrap(),
        config(),
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    // Seed the session row (as the runner/frontends do) so skill persists land.
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "plan-strip-lifecycle".into(),
            agent: Some("act".into()),
            model: Some("m".into()),
            created_at: 0,
            updated_at: 0,
            ..opencoder_store::SessionMeta::default()
        })
        .await
        .unwrap();
    session.set_skill(Some(plan_body()));
    // Real activations persist the body at consumption time
    // (run_with_registry / queue drains); mirror it so the store row
    // actually carries the skill when the abort hits.
    opencoder_session::skill_resolve::persist_active_skill(&session, &None).await;

    // Run 1: LLM failure -> aborted -> skill stays armed.
    mock.queue_script(vec![LlmEvent::Error("boom".into())]);
    let mut evs = Vec::new();
    opencoder_session::run(&mut session, "plan it".into(), |ev| evs.push(ev))
        .await
        .unwrap_err();
    assert!(
        session.skill_prompt_cloned().is_some(),
        "aborted run keeps the skill armed"
    );
    assert!(
        store
            .get_session("plan-strip-lifecycle")
            .await
            .unwrap()
            .and_then(|m| m.skill)
            .is_some(),
        "store row keeps the skill for resume"
    );

    // Run 2 (continued task): STILL plans without advertising build...
    mock.queue_script(vec![done_turn()]);
    let reqs_before = mock.requests().len();
    opencoder_session::run(&mut session, "keep planning".into(), |ev| evs.push(ev))
        .await
        .unwrap();
    let req = &mock.requests()[reqs_before];
    let system = req.messages[0]["content"].as_str().unwrap();
    assert!(
        !system.contains(BUILD_CLAUSE),
        "continued task-plan run must not advertise build, got: {system}"
    );

    // ...but its COMPLETION clears the skill (chip green)...
    assert!(
        session.skill_prompt_cloned().is_none(),
        "completed run clears the skill"
    );
    assert!(
        store
            .get_session("plan-strip-lifecycle")
            .await
            .unwrap()
            .and_then(|m| m.skill)
            .is_none(),
        "store row cleared on completion"
    );

    // ...and the NEXT run advertises build again.
    mock.queue_script(vec![done_turn()]);
    let reqs_before = mock.requests().len();
    opencoder_session::run(&mut session, "now implement".into(), |ev| evs.push(ev))
        .await
        .unwrap();
    let req = &mock.requests()[reqs_before];
    assert!(
        req.messages[0]["content"]
            .as_str()
            .unwrap()
            .contains(BUILD_CLAUSE),
        "post-completion runs advertise build again"
    );
}
