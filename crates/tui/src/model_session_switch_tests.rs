use std::sync::Arc;

use opencoder_core::{Config, ConfigMode, HttpHeader, ProviderConfig};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::SessionModelOverrides;
use tokio::sync::mpsc;

use super::*;

fn config() -> Config {
    Config {
        model: "openai/shared".into(),
        provider: ProviderConfig {
            base_url: "https://example.invalid/v1".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn chat(config: &Config) -> ChatView {
    ChatView {
        shared_model: config.model.clone(),
        ..Default::default()
    }
}

fn markers(chat: &ChatView) -> Vec<String> {
    chat.blocks
        .iter()
        .filter_map(|block| match block {
            crate::chat::ChatBlock::Marker(lines) => Some(
                lines
                    .iter()
                    .flat_map(|line| line.spans.iter())
                    .map(|span| span.content.as_ref())
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

async fn acknowledged_switch(
    patch: serde_json::Value,
    config: &mut Config,
    client: &mut Arc<dyn ChatStream>,
    tx: &mpsc::Sender<UiCmd>,
    rx: &mut mpsc::Receiver<UiCmd>,
    chat: &mut ChatView,
    result: Result<(), String>,
) -> UiCmd {
    let ((), command) = tokio::join!(switch_session(patch, config, client, tx, chat), async {
        let command = rx.recv().await.expect("session command");
        match &command {
            UiCmd::ApplySession { ack, .. } => {
                ack.send(result).expect("completion receiver");
            }
            other => panic!("unexpected command: {other:?}"),
        }
        command
    });
    command
}

#[tokio::test]
async fn act_selection_commits_after_ack_without_rebuilding_client() {
    let mut config = config();
    let mut chat = chat(&config);
    chat.agent = "act".into();
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let original = Arc::as_ptr(&client);
    let (tx, mut rx) = mpsc::channel(2);

    let command = acknowledged_switch(
        serde_json::json!({"modes":{"act":{"model":"two/act"}}}),
        &mut config,
        &mut client,
        &tx,
        &mut rx,
        &mut chat,
        Ok(()),
    )
    .await;

    assert!(matches!(
        command,
        UiCmd::ApplySession {
            selection: Some((ConfigMode::Act, ref model)),
            provider_changed: false,
            ..
        } if model == "two/act"
    ));
    assert_eq!(Arc::as_ptr(&client), original);
    assert_eq!(config.model, "openai/shared");
    assert_eq!(chat.shared_model, "openai/shared");
    assert_eq!(chat.model, "two/act");
    assert_eq!(
        chat.model_overrides.unwrap().act.as_deref(),
        Some("two/act")
    );
}

#[tokio::test]
async fn plan_selection_keeps_the_shared_overlay_without_rebuilding_client() {
    let mut config = config();
    config.modes.shared = opencoder_core::ModeConfig {
        model: Some("openai/config-shared".into()),
        max_tokens: Some(4_096),
        ..Default::default()
    };
    let empty = SessionModelOverrides::default();
    let mut chat = ChatView {
        agent: "plan".into(),
        shared_model: crate::model_scope::shared_model(&config, Some(&empty)),
        model: "openai/config-shared".into(),
        model_overrides: Some(empty),
        ..Default::default()
    };
    assert_eq!(chat.shared_model, "openai/config-shared");
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let original = Arc::as_ptr(&client);
    let (tx, mut rx) = mpsc::channel(2);

    let command = acknowledged_switch(
        serde_json::json!({"modes":{"plan":{"model":"openai/session-plan"}}}),
        &mut config,
        &mut client,
        &tx,
        &mut rx,
        &mut chat,
        Ok(()),
    )
    .await;

    assert!(matches!(
        command,
        UiCmd::ApplySession {
            selection: Some((ConfigMode::Plan, ref model)),
            provider_changed: false,
            ..
        } if model == "openai/session-plan"
    ));
    assert_eq!(Arc::as_ptr(&client), original);
    assert_eq!(config.model, "openai/shared");
    assert_eq!(
        chat.shared_model, "openai/config-shared",
        "a Plan pick never touches the Shared slot"
    );
    assert_eq!(chat.model, "openai/session-plan");
    assert_eq!(
        chat.model_overrides,
        Some(SessionModelOverrides {
            plan: Some("openai/session-plan".into()),
            ..Default::default()
        })
    );
    assert_eq!(
        crate::model_scope::effective_for_chat(&config, &chat).max_tokens,
        Some(4_096),
        "the Plan pick inherits Shared generation overlays"
    );
    // The Shared default in a patch is never a session selection.
    let mut chat = chat.clone();
    // A bare top-level `model` is the onboarding default, never a session
    // selection: nothing is sent to the worker and the call returns at once.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        switch_session(
            serde_json::json!({"model":"openai/not-a-pick"}),
            &mut config,
            &mut client,
            &tx,
            &mut chat,
        ),
    )
    .await
    .expect("a patch without a selection returns without waiting for an ack");
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    assert_eq!(Arc::as_ptr(&client), original);
    assert_eq!(chat.model, "openai/session-plan", "display unchanged");
    let lines = markers(&chat);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("contained neither Provider data nor a model selection")),
        "{lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("switched"))
            .count(),
        1,
        "only the Plan pick above produced a switch marker: {lines:?}"
    );
}

#[tokio::test]
async fn provider_add_replaces_fields_and_rebuilds_only_after_ack() {
    let mut config = config();
    config.providers.insert(
        "old".into(),
        ProviderConfig {
            base_url: "https://example.invalid/old".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
    );
    let mut chat = chat(&config);
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let original = Arc::as_ptr(&client);
    let (tx, mut rx) = mpsc::channel(2);
    let patch = serde_json::json!({
        "modes": {"act": {"model": "demo/session-model"}},
        "providers": {
            "demo": {
                "base_url":"https://example.invalid/v2",
                "api_key": null,
                "model":"session-model",
                "headers":[{"name":"x-new","value":"new"}]
            },
            "old": null
        }
    });

    let command = acknowledged_switch(
        patch,
        &mut config,
        &mut client,
        &tx,
        &mut rx,
        &mut chat,
        Ok(()),
    )
    .await;

    assert!(matches!(
        command,
        UiCmd::ApplySession {
            provider_changed: true,
            ..
        }
    ));
    assert_ne!(Arc::as_ptr(&client), original);
    assert_eq!(config.model, "openai/shared");
    assert_eq!(chat.shared_model, "openai/shared", "Shared is not a pick");
    assert_eq!(
        chat.model_overrides
            .as_ref()
            .and_then(|items| items.act.as_deref()),
        Some("demo/session-model")
    );
    assert!(!config.providers.contains_key("old"));
    assert_eq!(config.providers["demo"].api_key, None);
    assert_eq!(
        config.providers["demo"].headers,
        vec![HttpHeader {
            name: "x-new".into(),
            value: "new".into(),
        }]
    );
    assert!(markers(&chat)
        .iter()
        .any(|line| line.contains("Provider + Act") && line.contains("session only")));
}

#[tokio::test]
async fn unchanged_provider_patch_does_not_rebuild_client() {
    let mut config = config();
    config.providers.insert(
        "demo".into(),
        ProviderConfig {
            base_url: "https://example.invalid/v1".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
    );
    let before = config.clone();
    let mut chat = chat(&config);
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let original = Arc::as_ptr(&client);
    let (tx, mut rx) = mpsc::channel(2);

    let command = acknowledged_switch(
        serde_json::json!({"providers":{"demo":{
            "base_url":"https://example.invalid/v1",
            "api_key":"placeholder"
        }}}),
        &mut config,
        &mut client,
        &tx,
        &mut rx,
        &mut chat,
        Ok(()),
    )
    .await;
    assert!(matches!(
        command,
        UiCmd::ApplySession {
            provider_changed: false,
            ..
        }
    ));
    assert_eq!(Arc::as_ptr(&client), original);
    assert_eq!(config.providers, before.providers);
}

#[tokio::test]
async fn send_failure_and_worker_failure_leave_all_outer_state_unchanged() {
    for close_channel in [true, false] {
        let mut config = config();
        let before = config.clone();
        let mut chat = chat(&config);
        let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        let original = Arc::as_ptr(&client);
        let (tx, mut rx) = mpsc::channel(2);
        if close_channel {
            drop(rx);
            switch_session(
                serde_json::json!({"modes":{"act":{"model":"openai/rejected"}}}),
                &mut config,
                &mut client,
                &tx,
                &mut chat,
            )
            .await;
        } else {
            acknowledged_switch(
                serde_json::json!({"modes":{"act":{"model":"openai/rejected"}}}),
                &mut config,
                &mut client,
                &tx,
                &mut rx,
                &mut chat,
                Err("store unavailable".into()),
            )
            .await;
        }
        assert_eq!(config.model, before.model);
        assert_eq!(chat.shared_model, before.model);
        assert_eq!(chat.model, ChatView::default().model);
        assert_eq!(Arc::as_ptr(&client), original);
        let text = markers(&chat).join("\n");
        assert!(text.contains("failed"), "{text}");
        assert!(!text.contains("applied"), "{text}");
    }
}

#[tokio::test]
async fn out_of_order_completions_do_not_cross_commit_or_rollback() {
    let (tx, mut rx) = mpsc::channel(4);
    let spawn = |model: &'static str, tx: mpsc::Sender<UiCmd>| {
        tokio::spawn(async move {
            let mut config = config();
            let mut chat = chat(&config);
            let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
            switch_session(
                serde_json::json!({"modes":{"act":{"model":model}}}),
                &mut config,
                &mut client,
                &tx,
                &mut chat,
            )
            .await;
            (config, chat)
        })
    };
    let first = spawn("openai/first", tx.clone());
    let second = spawn("openai/second", tx);
    let a = rx.recv().await.unwrap();
    let b = rx.recv().await.unwrap();
    let selected = |command: &UiCmd| match command {
        UiCmd::ApplySession {
            selection: Some((ConfigMode::Act, selected)),
            ..
        } => selected.clone(),
        _ => panic!("mis-correlated test command"),
    };
    let (second_command, first_command) = if selected(&a) == "openai/second" {
        (&a, &b)
    } else {
        assert_eq!(selected(&a), "openai/first");
        assert_eq!(selected(&b), "openai/second");
        (&b, &a)
    };
    // Complete the newer operation first and fail the older one afterwards,
    // regardless of the order in which the two tasks entered the channel.
    for (command, result) in [
        (second_command, Ok(())),
        (first_command, Err("unrelated store error".into())),
    ] {
        let UiCmd::ApplySession { ack, .. } = command else {
            unreachable!()
        };
        ack.send(result).unwrap();
    }
    let (first_config, first_chat) = first.await.unwrap();
    let (second_config, second_chat) = second.await.unwrap();
    assert_eq!(first_config.model, "openai/shared");
    assert_eq!(second_config.model, "openai/shared");
    assert_eq!(first_chat.shared_model, "openai/shared");
    assert_eq!(second_chat.shared_model, "openai/shared");
    assert_eq!(
        second_chat
            .model_overrides
            .as_ref()
            .and_then(|items| items.act.as_deref()),
        Some("openai/second")
    );
    assert!(
        first_chat.model_overrides.is_none(),
        "the failed older pick commits nothing: {:?}",
        first_chat.model_overrides
    );
    assert!(!markers(&first_chat).join("").contains("switched"));
    assert!(markers(&second_chat).join("").contains("switched"));
}

#[path = "model_session_switch_tests/legacy_conversion.rs"]
mod legacy_conversion;
