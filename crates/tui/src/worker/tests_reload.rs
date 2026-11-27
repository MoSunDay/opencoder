use super::*;

fn full_reload(config: Config) -> UiCmd {
    UiCmd::ReloadConfig {
        config: Box::new(config),
        scope: ConfigReloadScope::Full,
        ack: None,
    }
}

#[tokio::test]
async fn reload_config_success_swaps_model() {
    use opencoder_core::{resolve_agent, Config, ProviderConfig};
    use opencoder_llm::MockChatClient;

    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let agent = resolve_agent("act").expect("act agent");
    let mut sess = SessionState::new(
        "reload-ok",
        agent,
        Config::default(),
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    );
    assert_eq!(sess.model, "gpt-4o-mini", "default model id");

    let new_cfg = Config {
        model: "openai/test-model".into(),
        provider: ProviderConfig {
            api_key: Some("k".into()),
            ..Default::default()
        },
        ..Config::default()
    };
    let (command, mut completion) = acknowledged_config_reload(new_cfg, ConfigReloadScope::Full);
    let should_break = process_cmd(command, &mut sess, &evt_tx, 0).await;
    assert_eq!(completion.recv().await, Some(Ok(())));
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    assert_eq!(sess.model, "test-model", "model must be swapped on success");
}

/// Config saves that keep the model unchanged land here: no `[model]`
/// marker and no store rewrite may fire for them.
#[tokio::test]
async fn reload_config_same_model_emits_no_model_switch() {
    use opencoder_core::{resolve_agent, Config, ProviderConfig};
    use opencoder_llm::MockChatClient;

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let agent = resolve_agent("act").expect("act agent");
    let mut sess = SessionState::new(
        "reload-same-model",
        agent,
        Config::default(),
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    );
    let prev_model = sess.config.model.clone();

    // Same model, resolvable endpoint (api_key present): nothing but the
    // model identity changes — no ModelSwitch may fire.
    let new_cfg = Config {
        provider: ProviderConfig {
            api_key: Some("k".into()),
            ..Default::default()
        },
        ..Config::default()
    };
    let should_break = process_cmd(full_reload(new_cfg), &mut sess, &evt_tx, 0).await;
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    assert_eq!(
        sess.config.model, prev_model,
        "model must be unchanged on same-model reload"
    );
    assert!(
        evt_rx.try_recv().is_err(),
        "no ModelSwitch/Error event may fire when the model did not change"
    );
}

#[tokio::test]
async fn reload_config_bad_proxy_rejects_reload_and_keeps_previous_runtime() {
    use opencoder_core::{resolve_agent, Config, NetworkConfig, ProviderConfig};
    use opencoder_llm::MockChatClient;

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(16);
    let agent = resolve_agent("act").expect("act agent");
    let mut sess = SessionState::new(
        "reload-bad-proxy",
        agent,
        Config::default(),
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    );
    assert_eq!(sess.model, "gpt-4o-mini", "default model id");

    // api_key present so resolve_endpoint() succeeds, but an invalid
    // proxy URL ("://nope") makes ChatClient::new fail before mutation.
    let new_cfg = Config {
        model: "openai/proxy-model".into(),
        provider: ProviderConfig {
            api_key: Some("k".into()),
            ..Default::default()
        },
        network: NetworkConfig {
            proxy: Some("://nope".into()),
        },
        ..Config::default()
    };
    let should_break = process_cmd(full_reload(new_cfg), &mut sess, &evt_tx, 0).await;
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    // Client construction fails before Store or SessionState mutation.
    assert_eq!(sess.config.model, "openai/gpt-4o-mini");
    assert_eq!(sess.effective_model(), "openai/gpt-4o-mini");
    let ev = evt_rx.recv().await.expect("an error event was forwarded");
    match ev {
        UiEvent::Session(SessionEvent::Error(msg)) => {
            assert!(
                msg.contains("client build failed"),
                "unexpected error message: {msg}"
            );
        }
        other => panic!(
            "expected Error event, got a different variant: {}",
            variant_name(&other)
        ),
    }
}

#[tokio::test]
async fn reload_config_missing_api_key_rejects_reload_and_keeps_previous_runtime() {
    use opencoder_core::{resolve_agent, scoped_config_home, Config, ProviderConfig};
    use opencoder_llm::MockChatClient;

    // Isolate env on this thread only: `resolve_endpoint`'s `OPENAI_API_KEY`
    // fallback must see no host key, so the missing-api_key config fails
    // deterministically. No `std::env::remove_var` (thread-unsafe → UB).
    let _iso = scoped_config_home(std::env::temp_dir());

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(16);
    let agent = resolve_agent("act").expect("act agent");
    let mut sess = SessionState::new(
        "reload-no-key",
        agent,
        Config::default(),
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    );
    assert_eq!(sess.model, "gpt-4o-mini", "default model id");

    // api_key missing so resolve_endpoint() fails before any client
    // construction, Store write, or SessionState mutation.
    let new_cfg = Config {
        model: "openai/no-key-model".into(),
        provider: ProviderConfig {
            api_key: None,
            ..Default::default()
        },
        ..Config::default()
    };
    let should_break = process_cmd(full_reload(new_cfg), &mut sess, &evt_tx, 0).await;
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    // Endpoint resolution also fails before any Store/config mutation.
    assert_eq!(sess.config.model, "openai/gpt-4o-mini");
    assert_eq!(sess.effective_model(), "openai/gpt-4o-mini");
    let ev = evt_rx.recv().await.expect("an error event was forwarded");
    match ev {
        UiEvent::Session(SessionEvent::Error(msg)) => {
            assert!(
                msg.contains("endpoint resolve failed"),
                "unexpected error message: {msg}"
            );
            assert!(
                !msg.contains("client build failed"),
                "must not mention client build failure: {msg}"
            );
        }
        other => panic!(
            "expected Error event, got a different variant: {}",
            variant_name(&other)
        ),
    }
}

#[tokio::test]
async fn reload_config_compares_the_current_agents_full_effective_model() {
    use opencoder_core::{resolve_agent, Config, ModeConfig, ModesConfig, ProviderConfig};
    use opencoder_llm::MockChatClient;

    let config = |act_model: &str, max_tokens| Config {
        model: "openai/shared".into(),
        provider: ProviderConfig {
            api_key: Some("test-placeholder".into()),
            ..Default::default()
        },
        modes: ModesConfig {
            act: Some(ModeConfig {
                model: Some(act_model.into()),
                max_tokens: Some(max_tokens),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut sess = SessionState::new(
        "reload-effective-model",
        resolve_agent("act").unwrap(),
        config("openai/same-id", 100),
        std::sync::Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    );
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);

    process_cmd(
        full_reload(config("deepseek/same-id", 100)),
        &mut sess,
        &evt_tx,
        0,
    )
    .await;

    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "deepseek/same-id");
    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ModelSwitch(model))) if model == "same-id"
    ));
    assert!(sess.take_model_switch().is_none());

    process_cmd(
        full_reload(config("deepseek/same-id", 200)),
        &mut sess,
        &evt_tx,
        0,
    )
    .await;
    assert_eq!(sess.effective_config().max_tokens, Some(200));
    assert!(
        evt_rx.try_recv().is_err(),
        "parameter-only effective config changes must not emit ModelSwitch"
    );
}

fn variant_name(ev: &UiEvent) -> &'static str {
    match ev {
        UiEvent::Session(_) => "Session",
        UiEvent::AssistantFinal(_) => "AssistantFinal",
        UiEvent::TurnDone(..) => "TurnDone",
        UiEvent::RemoteSnapshot { .. } => "RemoteSnapshot",
        UiEvent::SubtaskUpdate { .. } => "SubtaskUpdate",
    }
}
