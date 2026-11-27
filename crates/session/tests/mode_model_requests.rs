use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ConfigMode, ModeConfig, ModesConfig};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_session::compaction::exceeds_hard_limit;
use opencoder_session::model::{config_mode_for_agent, effective_config_for};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::SessionModelOverrides;

fn done() -> LlmEvent {
    LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: None,
    }
}

fn config() -> Config {
    Config {
        model: "base/provider-model".into(),
        max_tokens: Some(50),
        context_limit: Some(8_000),
        modes: ModesConfig {
            shared: ModeConfig {
                model: Some("shared/config-model".into()),
                reasoning_effort: Some("low".into()),
                max_tokens: Some(100),
                context_limit: Some(7_000),
                cache_salt: Some(false),
                ..Default::default()
            },
            act: Some(ModeConfig {
                model: Some("provider-a/act-model".into()),
                reasoning_effort: Some("high".into()),
                interleaved_thinking: Some(false),
                max_tokens: Some(200),
                context_limit: Some(6_000),
                cache_salt: Some(true),
            }),
            plan: Some(ModeConfig {
                model: Some("provider-b/plan-model".into()),
                reasoning_effort: Some("medium".into()),
                interleaved_thinking: Some(true),
                max_tokens: Some(300),
                context_limit: Some(5_000),
                cache_salt: Some(false),
            }),
        },
        ..Default::default()
    }
}

fn session(config: Config, mock: Arc<MockChatClient>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let client: Arc<dyn ChatStream> = mock;
    let state = SessionState::new(
        "mode-request-test",
        resolve_agent("act").unwrap(),
        config,
        client,
        dir.path().to_path_buf(),
    );
    (dir, state)
}

async fn run_turn(session: &mut SessionState, text: &str) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    run(session, text.into(), |event| events.push(event))
        .await
        .unwrap();
    events
}

fn assert_switch_precedes_round(events: &[SessionEvent], model: &str) {
    let switch = events
        .iter()
        .position(|event| matches!(event, SessionEvent::ModelSwitch(value) if value == model))
        .unwrap();
    let round = events
        .iter()
        .position(|event| matches!(event, SessionEvent::LlmRoundStart { .. }))
        .unwrap();
    assert!(switch < round, "model event must precede its request round");
}

#[tokio::test]
async fn act_plan_act_requests_and_metadata_follow_each_effective_mode() {
    let mock = Arc::new(MockChatClient::new().with_default(vec![done()]));
    let (_dir, mut session) = session(config(), mock.clone());

    let first = run_turn(&mut session, "act one").await;
    assert!(!first
        .iter()
        .any(|e| matches!(e, SessionEvent::ModelSwitch(_))));
    let first_metadata = session.messages.last().unwrap().model.clone();
    session.agent = resolve_agent("plan").unwrap();
    let second = run_turn(&mut session, "plan").await;
    assert_switch_precedes_round(&second, "plan-model");
    let second_metadata = session.messages.last().unwrap().model.clone();
    session.agent = resolve_agent("act").unwrap();
    let third = run_turn(&mut session, "act two").await;
    assert_switch_precedes_round(&third, "act-model");
    let third_metadata = session.messages.last().unwrap().model.clone();

    let requests = mock.requests();
    let requests: Vec<_> = requests
        .iter()
        .filter(|request| request.purpose == opencoder_llm::RequestPurpose::Conversation)
        .collect();
    assert_eq!(requests.len(), 3);
    let expected = [
        (
            "provider-a/act-model",
            200,
            "high",
            Some("act:mode-request-test"),
        ),
        ("provider-b/plan-model", 300, "medium", None),
        (
            "provider-a/act-model",
            200,
            "high",
            Some("act:mode-request-test"),
        ),
    ];
    for (request, (model, max_tokens, effort, salt)) in requests.iter().zip(expected) {
        assert_eq!(request.model, model);
        assert_eq!(request.max_tokens, Some(max_tokens));
        assert_eq!(request.reasoning_effort.as_deref(), Some(effort));
        assert_eq!(request.cache_salt.as_deref(), salt);
    }

    let assistant_models =
        [first_metadata, second_metadata, third_metadata].map(|model| model.unwrap());
    assert_eq!(assistant_models, ["act-model", "plan-model", "act-model"]);
    assert_eq!(session.effective_context_limit(), 6_000);
}

#[tokio::test]
async fn explicit_model_wins_requests_without_losing_mode_generation_overlays() {
    let mock = Arc::new(MockChatClient::new().with_default(vec![done()]));
    let (_dir, mut session) = session(config(), mock.clone());
    assert!(session.apply_explicit_model("override/explicit-model"));
    assert_eq!(
        session.model_overrides(),
        Some(&SessionModelOverrides::default())
    );

    for (agent, max_tokens, effort, context_limit) in [
        ("act", 200, "high", 6_000),
        ("plan", 300, "medium", 5_000),
        ("build", 100, "low", 7_000),
    ] {
        session.agent = resolve_agent(agent).unwrap();
        let effective = session.effective_config();
        assert_eq!(effective.model, "override/explicit-model");
        assert_eq!(effective.max_tokens, Some(max_tokens));
        assert_eq!(effective.reasoning_effort.as_deref(), Some(effort));
        assert_eq!(effective.context_limit(), context_limit);
        run_turn(&mut session, agent).await;
    }

    let requests = mock.requests();
    let requests: Vec<_> = requests
        .iter()
        .filter(|request| request.purpose == opencoder_llm::RequestPurpose::Conversation)
        .collect();
    assert_eq!(requests.len(), 3);
    for (request, (max_tokens, effort)) in
        requests
            .iter()
            .zip([(200, "high"), (300, "medium"), (100, "low")])
    {
        assert_eq!(request.model, "override/explicit-model");
        assert_eq!(request.max_tokens, Some(max_tokens));
        assert_eq!(request.reasoning_effort.as_deref(), Some(effort));
    }
}

#[test]
fn hard_context_limit_tracks_the_current_mode() {
    let mock = Arc::new(MockChatClient::new());
    let (_dir, mut session) = session(config(), mock);
    session.last_usage.input_tokens = 5_500;
    assert!(!exceeds_hard_limit(&session));
    session.agent = resolve_agent("plan").unwrap();
    assert!(exceeds_hard_limit(&session));
    session.agent = resolve_agent("build").unwrap();
    assert!(!exceeds_hard_limit(&session));
}

#[test]
fn only_exact_builtin_primary_agents_use_mode_overlays() {
    let act = resolve_agent("act").unwrap();
    let plan = resolve_agent("plan").unwrap();
    let build = resolve_agent("build").unwrap();
    assert_eq!(config_mode_for_agent(&act), ConfigMode::Act);
    assert_eq!(config_mode_for_agent(&plan), ConfigMode::Plan);
    assert_eq!(config_mode_for_agent(&build), ConfigMode::Shared);

    let mut custom = act.clone();
    custom.name = "custom".into();
    assert_eq!(config_mode_for_agent(&custom), ConfigMode::Shared);
    assert_eq!(
        effective_config_for(&config(), &custom, None).model,
        "shared/config-model"
    );
}

#[test]
fn legacy_rows_keep_the_single_model_until_explicitly_converted() {
    let cfg = config();
    let act = resolve_agent("act").unwrap();
    assert_eq!(effective_config_for(&cfg, &act, None).model, cfg.model);
    assert_eq!(effective_config_for(&cfg, &act, None).max_tokens, Some(50));

    let empty = SessionModelOverrides::default();
    assert_eq!(
        effective_config_for(&cfg, &act, Some(&empty)).model,
        "provider-a/act-model"
    );
    let selected = SessionModelOverrides {
        act: Some("provider-c/session-act".into()),
        plan: None,
        shared: None,
    };
    let effective = effective_config_for(&cfg, &act, Some(&selected));
    assert_eq!(effective.model, "provider-c/session-act");
    assert_eq!(effective.max_tokens, Some(200));
    assert_eq!(effective.context_limit(), 6_000);
    assert_eq!(effective.interleaved_thinking, Some(false));
}
