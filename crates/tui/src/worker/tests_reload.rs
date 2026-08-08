use super::*;

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
    let should_break =
        process_cmd(UiCmd::ReloadConfig(Box::new(new_cfg)), &mut sess, &evt_tx).await;
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    assert_eq!(sess.model, "test-model", "model must be swapped on success");
}

/// `/ap` toggles and pure max_iterations saves land here with an unchanged
/// model: no `[model]` marker and no store rewrite may fire for them.
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

    // Same model, resolvable endpoint (api_key present): the only thing
    // that changes is e.g. autopilot.enabled — no ModelSwitch may fire.
    let new_cfg = Config {
        provider: ProviderConfig {
            api_key: Some("k".into()),
            ..Default::default()
        },
        ..Config::default()
    };
    let should_break =
        process_cmd(UiCmd::ReloadConfig(Box::new(new_cfg)), &mut sess, &evt_tx).await;
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
async fn reload_config_bad_proxy_keeps_client_and_emits_error() {
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
    // proxy URL ("://nope") makes ChatClient::new fail -> keep-client
    // fallback path.
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
    let should_break =
        process_cmd(UiCmd::ReloadConfig(Box::new(new_cfg)), &mut sess, &evt_tx).await;
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    // model updated via keep-client fallback (consistent with on-disk config)
    assert_eq!(
        sess.model, "proxy-model",
        "model updated despite client failure"
    );
    // an Error event must have been forwarded to the UI
    let ev = evt_rx.recv().await.expect("an error event was forwarded");
    match ev {
        UiEvent::Session(SessionEvent::Error(msg)) => {
            assert!(
                msg.contains("client build failed"),
                "unexpected error message: {msg}"
            );
            assert!(
                msg.contains("proxy-model"),
                "error should mention new model"
            );
        }
        other => panic!(
            "expected Error event, got a different variant: {}",
            variant_name(&other)
        ),
    }
}

#[tokio::test]
async fn reload_config_missing_api_key_keeps_client_and_emits_error() {
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

    // api_key missing so resolve_endpoint() fails -> outer Err branch ->
    // keep-client fallback path. No ChatClient::new call is attempted.
    let new_cfg = Config {
        model: "openai/no-key-model".into(),
        provider: ProviderConfig {
            api_key: None,
            ..Default::default()
        },
        ..Config::default()
    };
    let should_break =
        process_cmd(UiCmd::ReloadConfig(Box::new(new_cfg)), &mut sess, &evt_tx).await;
    assert!(!should_break, "ReloadConfig must not break the worker loop");
    // model updated via keep-client fallback (consistent with on-disk config)
    assert_eq!(
        sess.model, "no-key-model",
        "model updated despite resolve failure"
    );
    // an Error event must have been forwarded to the UI
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
            assert!(
                msg.contains("no-key-model"),
                "error should mention new model"
            );
        }
        other => panic!(
            "expected Error event, got a different variant: {}",
            variant_name(&other)
        ),
    }
}

fn variant_name(ev: &UiEvent) -> &'static str {
    match ev {
        UiEvent::Session(_) => "Session",
        UiEvent::TurnDone(_) => "TurnDone",
    }
}
