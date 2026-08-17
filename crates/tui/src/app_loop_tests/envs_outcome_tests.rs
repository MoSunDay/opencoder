//! Tests for `handle_envs_outcome` — env mutations (marker writes, dir
//! create/delete, capture) plus the `/model`-style full refresh (client
//! rebuild, labels, ReloadConfig). Every test isolates config discovery via
//! `scoped_config_home` so no host config or env interferes.

use std::sync::Arc;

use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::envs_menu::{EnvField, EnvNameForm, EnvsList, EnvsMenu};
use crate::worker::UiCmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;
use opencoder_llm::MockChatClient;

fn marker_text(chat: &ChatView) -> String {
    chat.blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Marker(lines) => Some(lines.as_slice()),
            _ => None,
        })
        .flat_map(|lines| lines.iter())
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect()
}

fn enter_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
}

fn down_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Down, KeyModifiers::empty())
}

/// Two-env world under an isolated home: global base + env `alpha` overriding
/// the model. Returns the home tempdir (keep alive for the test body).
fn env_world() -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());
    let g = home.path().join(".opencoder");
    std::fs::create_dir_all(g.join("envs/alpha")).unwrap();
    std::fs::write(
        g.join("config.json"),
        r#"{"model":"prov/base","provider":{"base_url":"https://g.example","api_key":"gk"},"theme":"dark"}"#,
    )
    .unwrap();
    std::fs::write(
        g.join("envs/alpha/config.json"),
        r#"{"model":"prov/alpha"}"#,
    )
    .unwrap();
    (home, work)
}

#[allow(clippy::too_many_arguments)]
async fn run_handler(
    envs_menu: &mut Option<EnvsMenu>,
    k: KeyEvent,
    workdir: &std::path::Path,
) -> (
    Arc<dyn opencoder_llm::ChatStream>,
    Config,
    String,
    ChatView,
    tokio::sync::mpsc::Receiver<UiCmd>,
) {
    let mut client: Arc<dyn opencoder_llm::ChatStream> = Arc::new(MockChatClient::new());
    let mut config = Config::default();
    let mut model_label = String::new();
    let mut ct = 0u64;
    let mut cl = 0u64;
    let mut frame_ms = 25u64;
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(frame_ms));
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut chat = ChatView::default();
    let _ = handle_envs_outcome(
        envs_menu,
        k,
        &mut client,
        &mut config,
        &mut model_label,
        &mut ct,
        &mut cl,
        &mut frame_ms,
        &mut ticker,
        &cmd_tx,
        &mut chat,
        workdir,
    )
    .await;
    (client, config, model_label, chat, cmd_rx)
}

/// Enter on an env row activates it: marker written, config/client/label
/// refreshed from the env layer, ReloadConfig dispatched, menu closed.
#[tokio::test]
async fn activate_env_refreshes_config_and_notifies_worker() {
    let (home, work) = env_world();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());

    let mut menu = Some(EnvsMenu::List(EnvsList {
        envs: vec!["alpha".into()],
        active: None,
        selected: 1,
        confirm_delete: None,
    }));
    let (_, config, model_label, chat, mut cmd_rx) =
        run_handler(&mut menu, enter_key(), work.path()).await;

    assert!(menu.is_none(), "activation closes the modal");
    assert_eq!(opencoder_core::active_env().as_deref(), Some("alpha"));
    assert_eq!(model_label, "prov/alpha", "label follows the env layer");
    assert_eq!(config.model, "prov/alpha");
    assert!(
        marker_text(&chat).contains("activated"),
        "activation marker"
    );
    assert!(matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))));
}

/// Enter on the base row while an env is active deactivates: marker cleared,
/// base config restored.
#[tokio::test]
async fn deactivate_via_base_row_restores_base_config() {
    let (home, work) = env_world();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());
    opencoder_core::set_active_env(Some("alpha")).unwrap();

    let mut menu = Some(EnvsMenu::List(EnvsList {
        envs: vec!["alpha".into()],
        active: Some("alpha".into()),
        selected: 0,
        confirm_delete: None,
    }));
    let (_, config, model_label, chat, mut cmd_rx) =
        run_handler(&mut menu, enter_key(), work.path()).await;

    assert!(menu.is_none());
    assert!(opencoder_core::active_env().is_none());
    assert_eq!(model_label, "prov/base", "base model restored");
    assert_eq!(config.model, "prov/base");
    assert!(marker_text(&chat).contains("deactivated"));
    assert!(matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))));
}

/// `n` → name form → Enter creates the env (default capture) and reopens the
/// list containing it; no ReloadConfig (creation never activates).
#[tokio::test]
async fn create_from_form_captures_and_reopens_list() {
    let (home, work) = env_world();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());

    let mut form = EnvNameForm::new(vec!["alpha".into()]);
    form.name = "beta".into();
    form.name_cursor = 4;
    form.field = EnvField::Name;
    let mut menu = Some(EnvsMenu::Form(form));

    let (_, _, _, chat, mut cmd_rx) = run_handler(&mut menu, enter_key(), work.path()).await;

    let dir = home.path().join(".opencoder/envs/beta");
    assert!(dir.is_dir(), "env dir created");
    let captured: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(
        captured["model"], "prov/base",
        "capture copies the base chain"
    );
    assert_eq!(captured["provider"]["api_key"], "gk");
    assert!(matches!(menu, Some(EnvsMenu::List(_))), "list reopened");
    assert!(marker_text(&chat).contains("created"));
    assert!(cmd_rx.try_recv().is_err(), "no ReloadConfig on create");
}

/// `d` + `y` deletes; deleting the ACTIVE env clears the marker and runs the
/// full refresh back to base.
#[tokio::test]
async fn delete_active_env_clears_marker_and_refreshes() {
    let (home, work) = env_world();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());
    opencoder_core::set_active_env(Some("alpha")).unwrap();

    let mut menu = Some(EnvsMenu::List(EnvsList {
        envs: vec!["alpha".into()],
        active: Some("alpha".into()),
        selected: 1,
        confirm_delete: Some(1),
    }));
    let (_, config, model_label, chat, mut cmd_rx) =
        run_handler(&mut menu, enter_key(), work.path()).await;

    assert!(!home.path().join(".opencoder/envs/alpha").exists());
    assert!(opencoder_core::active_env().is_none(), "marker cleared");
    assert_eq!(model_label, "prov/base", "refresh back to base");
    assert_eq!(config.model, "prov/base");
    assert!(marker_text(&chat).contains("deleted"));
    assert!(matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))));
}

/// `e` recaptures the base chain into the ACTIVE env → effective config
/// changes → full refresh. Non-active env recapture stays silent.
#[tokio::test]
async fn recapture_active_env_refreshes() {
    let (home, work) = env_world();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());
    // make the base differ from the stale env snapshot
    std::fs::write(
        home.path().join(".opencoder/config.json"),
        r#"{"model":"prov/base2","provider":{"base_url":"https://g.example","api_key":"gk"},"theme":"light"}"#,
    )
    .unwrap();
    opencoder_core::set_active_env(Some("alpha")).unwrap();

    let mut menu = Some(EnvsMenu::List(EnvsList {
        envs: vec!["alpha".into()],
        active: Some("alpha".into()),
        selected: 1,
        confirm_delete: None,
    }));
    let (_, config, model_label, chat, mut cmd_rx) = run_handler(
        &mut menu,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()),
        work.path(),
    )
    .await;

    assert_eq!(model_label, "prov/base2", "recaptured env follows new base");
    assert_eq!(config.model, "prov/base2");
    assert!(marker_text(&chat).contains("recaptured"));
    assert!(matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))));
    // the env snapshot on disk was replaced
    let snap: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".opencoder/envs/alpha/config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(snap["model"], "prov/base2");
}

/// Idle keys (navigation) must not reload anything.
#[tokio::test]
async fn navigation_is_idle_no_reload() {
    let (home, work) = env_world();
    let _iso = opencoder_core::scoped_config_home(home.path().to_path_buf());

    let mut menu = Some(EnvsMenu::List(EnvsList {
        envs: vec!["alpha".into()],
        active: None,
        selected: 0,
        confirm_delete: None,
    }));
    let (_, _, _, _, mut cmd_rx) = run_handler(&mut menu, down_key(), work.path()).await;
    assert!(matches!(menu, Some(EnvsMenu::List(_))));
    assert!(cmd_rx.try_recv().is_err(), "no UiCmd on navigation");
}
