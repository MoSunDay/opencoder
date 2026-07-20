//! Tests for SessionState config hot-reload, exercised by the TUI `/model`
//! menu via `UiCmd::ReloadConfig`. Covers two invariants:
//! - `apply_config_reload` swaps `client`/`model`/`config` in place (the
//!   current session routes through the new client).
//! - a fresh SessionState built with the rebuilt outer client + reloaded
//!   config (the `/task` switch path) also routes through the new client —
//!   the regression that the `mut client` fix in app.rs upholds.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_session::SessionState;

fn done() -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: "ok".into(),
        tool_calls: vec![],
        usage: None,
    }]
}

fn req() -> ChatRequest {
    ChatRequest {
        model: "m".into(),
        messages: vec![],
        tools: vec![],
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

fn cfg(model: &str) -> Config {
    Config {
        model: model.to_string(),
        reasoning_effort: Some("high".to_string()),
        ..Config::default()
    }
}

async fn drain(rx: tokio::sync::mpsc::Receiver<LlmEvent>) {
    let mut rx = rx;
    while rx.recv().await.is_some() {}
}

#[tokio::test]
async fn apply_config_reload_swaps_client_model_and_config() {
    let agent = resolve_agent("act").unwrap();
    let mock_a = Arc::new(MockChatClient::new().with_default(done()));
    let mock_b = Arc::new(MockChatClient::new().with_default(done()));

    let mut sess = SessionState::new(
        "s1",
        agent,
        cfg("old/model"),
        mock_a.clone() as Arc<dyn ChatStream>,
        "/tmp".into(),
    );

    // Pre-reload call routes through mock_a.
    drain(sess.client.chat_stream(req()).unwrap()).await;
    assert_eq!(mock_a.call_count(), 1);
    assert_eq!(mock_b.call_count(), 0);

    // Hot-reload to mock_b + new config.
    sess.apply_config_reload(cfg("new/model"), mock_b.clone() as Arc<dyn ChatStream>);

    // Fields updated.
    assert_eq!(
        sess.model, "model",
        "model must be derived from new config model_id"
    );
    assert_eq!(sess.config.model, "new/model");
    assert_eq!(sess.config.reasoning_effort.as_deref(), Some("high"));

    // Post-reload call routes through mock_b only.
    drain(sess.client.chat_stream(req()).unwrap()).await;
    assert_eq!(
        mock_a.call_count(),
        1,
        "old client must NOT serve after reload"
    );
    assert_eq!(mock_b.call_count(), 1, "new client must serve after reload");
}

#[tokio::test]
async fn fresh_session_after_reload_uses_new_client() {
    // Regression for the stale-client bug: after `/model` reload, the outer
    // `client` binding in run_app is rebuilt, so a `/task` new session built
    // from it must route through the NEW endpoint, not the startup one.
    let mock_old = Arc::new(MockChatClient::new().with_default(done()));
    let mock_new = Arc::new(MockChatClient::new().with_default(done()));

    // Simulate the run_app state after `/model` save rebuilt the outer client.
    let client_for_new_sessions: Arc<dyn ChatStream> = mock_new.clone();
    let reloaded_cfg = cfg("new/model");

    // `/task` switch constructs a fresh SessionState with current client + cfg.
    let agent = resolve_agent("act").unwrap();
    let new_sess = SessionState::new(
        "s2",
        agent,
        reloaded_cfg,
        client_for_new_sessions,
        "/tmp".into(),
    );

    drain(new_sess.client.chat_stream(req()).unwrap()).await;
    assert_eq!(
        mock_old.call_count(),
        0,
        "stale startup client must not serve new sessions"
    );
    assert_eq!(
        mock_new.call_count(),
        1,
        "rebuilt client must serve the new session"
    );
}

#[tokio::test]
async fn apply_config_reload_with_same_client_keeps_routing() {
    // apply_config_reload is also used to update only config/model while
    // passing the SAME client Arc (no endpoint change). Routing must continue.
    let agent = resolve_agent("act").unwrap();
    let mock = Arc::new(MockChatClient::new().with_default(done()));
    let mut sess = SessionState::new(
        "s1",
        agent,
        cfg("a/b"),
        mock.clone() as Arc<dyn ChatStream>,
        "/tmp".into(),
    );
    sess.apply_config_reload(cfg("c/d"), mock.clone() as Arc<dyn ChatStream>);
    assert_eq!(sess.model, "d");
    drain(sess.client.chat_stream(req()).unwrap()).await;
    assert_eq!(mock.call_count(), 1);
}

#[tokio::test]
async fn switch_provider_in_map_updates_model_and_client() {
    // When the config's model prefix switches to a different provider in the
    // `providers` map, apply_config_reload must update sess.model to the new
    // provider's model_id and route through the new client.
    use std::collections::HashMap;
    use opencoder_core::ProviderConfig;

    let agent = resolve_agent("act").unwrap();
    let mock_a = Arc::new(MockChatClient::new().with_default(done()));
    let mock_b = Arc::new(MockChatClient::new().with_default(done()));

    // Initial config: deepseek provider active.
    let mut providers_a = HashMap::new();
    providers_a.insert(
        "deepseek".to_string(),
        ProviderConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some("dk-key".to_string()),
            model: Some("deepseek-chat".to_string()),
        },
    );
    let cfg_a = Config {
        model: "deepseek/deepseek-chat".to_string(),
        providers: providers_a,
        ..Config::default()
    };

    let mut sess = SessionState::new(
        "s1",
        agent,
        cfg_a,
        mock_a.clone() as Arc<dyn ChatStream>,
        "/tmp".into(),
    );
    assert_eq!(sess.model, "deepseek-chat");

    // Pre-reload call routes through mock_a.
    drain(sess.client.chat_stream(req()).unwrap()).await;
    assert_eq!(mock_a.call_count(), 1);

    // New config: openai provider active.
    let mut providers_b = HashMap::new();
    providers_b.insert(
        "deepseek".to_string(),
        ProviderConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: Some("dk-key".to_string()),
            model: Some("deepseek-chat".to_string()),
        },
    );
    providers_b.insert(
        "openai".to_string(),
        ProviderConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some("oai-key".to_string()),
            model: Some("gpt-4o".to_string()),
        },
    );
    let cfg_b = Config {
        model: "openai/gpt-4o".to_string(),
        providers: providers_b,
        ..Config::default()
    };

    // Hot-reload: switch to mock_b + new config.
    sess.apply_config_reload(cfg_b, mock_b.clone() as Arc<dyn ChatStream>);

    // Model updated to the new provider's model_id.
    assert_eq!(sess.model, "gpt-4o");
    assert_eq!(sess.config.provider_id(), "openai");

    // Post-reload call routes through mock_b only.
    drain(sess.client.chat_stream(req()).unwrap()).await;
    assert_eq!(mock_a.call_count(), 1, "old client must NOT serve after switch");
    assert_eq!(mock_b.call_count(), 1, "new client must serve after switch");
}
