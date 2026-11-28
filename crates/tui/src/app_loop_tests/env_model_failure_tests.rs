//! Outer-state rollback checks for acknowledged local-Env activation.

use std::sync::Arc;

use super::HOME_TEST_LOCK;
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::command::EnvCommand;
use crate::model_menu::{EnvMenu, ModelMenu};
use crate::worker::UiCmd;
use opencoder_core::{create_config_env, Config};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::SessionModelOverrides;

fn marker_text(chat: &ChatView) -> String {
    chat.blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Marker(lines) => Some(lines.as_slice()),
            _ => None,
        })
        .flatten()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect()
}

async fn assert_failed_activation(
    workdir: &std::path::Path,
    initial: &Config,
    worker_failure: bool,
) {
    let mut config = initial.clone();
    let config_before = serde_json::to_value(&config).unwrap();
    let mut chat = ChatView {
        agent: "act".into(),
        shared_model: "openai/shared-temporary".into(),
        model: "openai/temporary".into(),
        model_overrides: Some(SessionModelOverrides {
            shared: Some("openai/shared-temporary".into()),
            act: Some("openai/temporary".into()),
            plan: Some("openai/plan-temporary".into()),
        }),
        ..Default::default()
    };
    let overrides_before = chat.model_overrides.clone();
    let mut menu = Some(ModelMenu::Env(EnvMenu::with_command(
        workdir,
        EnvCommand::Activate("alpha".into()),
    )));
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let original_client = Arc::as_ptr(&client);
    let mut threshold = 7_777;
    let mut context_limit = 8_888;
    let mut frame_ms = config.tui_frame_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let original_period = ticker.period();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(4);
    let model_switches = Default::default();

    let flow = if worker_failure {
        let (flow, ()) = tokio::join!(
            drain_pending_env(
                &mut menu,
                &mut client,
                &mut config,
                &mut threshold,
                &mut context_limit,
                &mut frame_ms,
                &mut ticker,
                &cmd_tx,
                &model_switches,
                &mut chat,
                workdir,
                None,
                false,
            ),
            async {
                let command = cmd_rx.recv().await.expect("ReloadEnv command");
                match command {
                    UiCmd::ReloadEnv { config, ack } => {
                        assert_eq!(config.model, "openai/env-shared");
                        ack.send(Err("injected Env worker failure".into())).unwrap();
                    }
                    other => panic!("unexpected command: {other:?}"),
                }
            }
        );
        flow
    } else {
        drop(cmd_rx);
        drain_pending_env(
            &mut menu,
            &mut client,
            &mut config,
            &mut threshold,
            &mut context_limit,
            &mut frame_ms,
            &mut ticker,
            &cmd_tx,
            &Default::default(),
            &mut chat,
            workdir,
            None,
            false,
        )
        .await
    };

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(menu.is_none());
    assert_eq!(serde_json::to_value(&config).unwrap(), config_before);
    assert_eq!(Arc::as_ptr(&client), original_client);
    assert_eq!(chat.shared_model, "openai/shared-temporary");
    assert_eq!(chat.model, "openai/temporary");
    assert_eq!(chat.model_overrides, overrides_before);
    assert_eq!(threshold, 7_777);
    assert_eq!(context_limit, 8_888);
    assert_eq!(frame_ms, config.tui_frame_ms());
    assert_eq!(ticker.period(), original_period);
    let text = marker_text(&chat);
    assert!(
        text.contains("[/env] activated Env `alpha` failed"),
        "{text}"
    );
    assert!(text.contains("runtime unchanged"), "{text}");
    if worker_failure {
        assert!(text.contains("injected Env worker failure"), "{text}");
    } else {
        assert!(text.contains("worker command channel closed"), "{text}");
    }
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn env_send_and_worker_failures_leave_outer_runtime_unchanged() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let workdir = root.path().join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::json!({
            "model": "openai/base-model",
            "providers": {"openai": {
                "base_url": "https://example.invalid/v1",
                "api_key": "test-placeholder",
                "model": "base-model"
            }}
        })
        .to_string(),
    )
    .unwrap();
    let _isolation = opencoder_core::scoped_config_home(home.clone());
    create_config_env("alpha").unwrap();
    std::fs::write(
        home.join(".opencoder/envs/alpha/config.json"),
        serde_json::json!({
            "model": "openai/env-shared",
            "context_limit": 12_000,
            "compaction": {"context_threshold": 9_000},
            "fps": 20,
            "modes": {"act": {"model": "openai/env-act", "context_limit": 11_000}}
        })
        .to_string(),
    )
    .unwrap();

    let base_config = Config::load(&workdir).unwrap();
    assert_failed_activation(&workdir, &base_config, false).await;
    assert_failed_activation(&workdir, &base_config, true).await;
}
