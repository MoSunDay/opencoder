//! Integration tests for the optional-capability surface:
//!
//! 1. **Capability gating** — `config.capabilities.tool_enabled` filters which
//!    tool schemas reach the LLM. The `tools` subagent allows `computer_use`,
//!    so toggling `capabilities.computer_use` must add/remove the
//!    `computer_use` schema from the request body built by the runner.
//! 2. **`tools` subagent dispatch** — the umbrella `tools` subagent must be
//!    dispatchable from the act agent (it is plan-visible, unlike `build`).
//! 3. **`/config` patch round-trip** — a capabilities patch saved via
//!    `Config::save` must be read back by `Config::load`.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            ..Default::default()
        }),
    }
}

/// A `task` tool call delegating to a subagent of the given type.
fn task_turn(subagent_type: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: format!("delegating to {subagent_type}"),
        tool_calls: vec![CompletedToolCall {
            id: "task-1".into(),
            name: "task".into(),
            input: serde_json::json!({
                "prompt": "do the thing",
                "subagent_type": subagent_type,
            }),
        }],
        usage: None,
    }
}

fn base_config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// Collect the tool-function names exposed in a request's `tools` schema list.
fn exposed_tool_names(req: &opencoder_llm::ChatRequest) -> Vec<String> {
    req.tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().map(String::from))
        .collect()
}

#[tokio::test]
async fn capability_gate_hides_computer_use_when_disabled() {
    // The `tools` subagent allows `computer_use`, but with the capability
    // disabled the runner's schema filter must drop it from the request while
    // keeping the always-on read-only filesystem tools.
    let mock = Arc::new(MockChatClient::new().with_default(vec![done_turn("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = base_config();
    cfg.capabilities.computer_use = false;
    let agent = resolve_agent("tools").expect("tools subagent registered");
    let mut s = SessionState::new("cap-off", agent, cfg, client, dir.path().to_path_buf());

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(!reqs.is_empty(), "at least one LLM request expected");
    let names = exposed_tool_names(&reqs[0]);
    assert!(
        !names.iter().any(|n| n == "computer_use"),
        "computer_use must be hidden when capability is disabled, got: {names:?}"
    );
    for required in ["read", "glob", "grep", "ls"] {
        assert!(
            names.iter().any(|n| n == required),
            "always-on tool '{required}' must remain exposed, got: {names:?}"
        );
    }
}

#[tokio::test]
async fn capability_gate_exposes_computer_use_when_enabled() {
    let mock = Arc::new(MockChatClient::new().with_default(vec![done_turn("ok")]));
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = base_config();
    cfg.capabilities.computer_use = true;
    let agent = resolve_agent("tools").expect("tools subagent registered");
    let mut s = SessionState::new("cap-on", agent, cfg, client, dir.path().to_path_buf());

    run(&mut s, "go".into(), |_| {}).await.unwrap();

    let reqs = mock.requests();
    assert!(!reqs.is_empty());
    let names = exposed_tool_names(&reqs[0]);
    assert!(
        names.iter().any(|n| n == "computer_use"),
        "computer_use must be exposed when capability is enabled, got: {names:?}"
    );
}

#[tokio::test]
async fn tools_subagent_is_dispatchable_from_act() {
    // The act agent delegates to the `tools` subagent. Unlike `build` (which
    // is plan-hidden), `tools` must dispatch cleanly: no "Unknown
    // subagent_type" error, and the subagent must make its own LLM call.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("tools")]) // act delegates
            .push_script(vec![done_turn("done")]) // tools subagent turn
            .push_script(vec![done_turn("final")]), // act final turn
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let mut cfg = base_config();
    cfg.capabilities.computer_use = true;
    cfg.capabilities.tools_subagent = true;
    let mut s = SessionState::new("dispatch", agent, cfg, client, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut s, "delegate to tools".into(), |ev| events.push(ev)).await.unwrap();

    // The task tool must complete without error and NOT report an unknown type.
    let task_ends: Vec<&SessionEvent> = events
        .iter()
        .filter(|ev| matches!(ev, SessionEvent::ToolEnd { name, .. } if name == "task"))
        .collect();
    assert!(!task_ends.is_empty(), "expected a task ToolEnd");
    for ev in &task_ends {
        if let SessionEvent::ToolEnd { is_error, output, .. } = ev {
            assert!(!*is_error, "tools subagent must dispatch cleanly");
            assert!(
                !output.contains("Unknown subagent_type"),
                "tools must be a known subagent, got: {output}"
            );
        }
    }

    // The subagent turn makes its own LLM call, so we expect >= 2 requests.
    let reqs = mock.requests();
    assert!(
        reqs.len() >= 2,
        "expected act + tools subagent requests, got {}",
        reqs.len()
    );
}

#[tokio::test]
async fn tools_subagent_rejected_when_capability_disabled() {
    // Defense-in-depth: even if the model emits a `tools` subagent call while
    // the capability switch is off, the runtime guard in `run_subagent` must
    // reject it with an "Unknown subagent_type 'tools'" error whose
    // valid-options list never advertises 'tools'.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("tools")]) // act tries to delegate
            .push_script(vec![done_turn("final")]), // act recovers
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let cfg = base_config(); // tools_subagent = false (default) -> capability OFF
    let mut s = SessionState::new("guard-off", agent, cfg, client, dir.path().to_path_buf());

    let mut events = Vec::new();
    run(&mut s, "delegate to tools".into(), |ev| events.push(ev))
        .await
        .unwrap();

    let err_output = events
        .iter()
        .find_map(|ev| match ev {
            SessionEvent::ToolEnd {
                name, is_error, output, ..
            } if name == "task" && *is_error => Some(output.clone()),
            _ => None,
        })
        .expect("expected an errored task ToolEnd rejecting the tools subagent");

    assert!(
        err_output.contains("Unknown subagent_type 'tools'"),
        "guard must name the rejected type, got: {err_output}"
    );
    assert!(
        !err_output.contains("'tools' (browser"),
        "valid-options list must not advertise tools when capability off, got: {err_output}"
    );
}

#[tokio::test]
async fn config_save_load_round_trips_capabilities() {
    // The `/config` save path: a ModelPatch.to_json() capabilities object is
    // merged via Config::save and must be read back by Config::load.
    let dir = tempfile::tempdir().unwrap();
    let patch = serde_json::json!({
        "model": "m/g",
        "capabilities": { "browser": true, "computer_use": true, "tools_subagent": true },
    });
    Config::save(dir.path(), &patch).expect("save patch");

    let loaded = Config::load(dir.path()).expect("load");
    assert!(loaded.capabilities.browser, "browser capability round-trips");
    assert!(loaded.capabilities.computer_use, "computer_use capability round-trips");
    assert!(loaded.capabilities.tools_subagent, "tools_subagent capability round-trips");

    // Toggle off and confirm the merge overwrites (not just creates).
    Config::save(
        dir.path(),
        &serde_json::json!({ "capabilities": { "browser": false, "computer_use": false, "tools_subagent": false } }),
    )
    .expect("save toggle");
    let reloaded = Config::load(dir.path()).expect("reload");
    assert!(!reloaded.capabilities.browser, "browser capability toggles off");
    assert!(!reloaded.capabilities.computer_use, "computer_use toggles off");
    assert!(!reloaded.capabilities.tools_subagent, "tools_subagent toggles off");
}
