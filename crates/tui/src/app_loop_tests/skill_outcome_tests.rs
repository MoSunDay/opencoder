//! Tests for `handle_skill_outcome` — the `/skill` modal's persistence layer.
//! The toggle patch (`{"skills":{<name>:{"enabled":<bool>}}}`) must land in
//! the `skills.json` domain file (global under the scoped home when no file
//! pre-exists), and `config.json` must not gain a `skills` key.

use std::path::PathBuf;

use super::{auto_ack_commands, HOME_TEST_LOCK};
use crate::app::app_loop::*;
use crate::chat::{ChatBlock, ChatView};
use crate::skill_menu::{SkillList, SkillMenu};
use crate::worker::UiCmd;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{Config, Skill};

/// Parse a JSON file from disk (missing file panics with the io error).
fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

/// Collect all marker-block text into a flat `String` for substring asserts.
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

/// One discovered skill (OFF by default: it is absent from the config).
fn discovered_skill() -> Skill {
    Skill {
        name: "alpha".into(),
        description: "test skill".into(),
        body: "body text".into(),
        source: PathBuf::from("/skills/alpha/SKILL.md"),
    }
}

/// Toggling a discovered skill via the `/skill` menu writes `skills.json`
/// under the scoped home (no pre-existing files → global write target) and
/// never creates `config.json` or puts a `skills` key into it. The reloaded
/// config carries the toggle and `ReloadConfig` is dispatched.
#[tokio::test]
async fn skill_toggle_writes_skills_domain_file_not_config_json() {
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    // Distinct project dir so the project candidate (`project/.opencoder/
    // skills.json`) and the global one differ — proving the write went global.
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let global_skills = tmp.path().join(".opencoder").join("skills.json");

    let mut config = Config::default();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
    let mut cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView::default();
    let mut skill_menu = Some(SkillMenu::List(SkillList::from_discovered(
        &[discovered_skill()],
        &config,
    )));

    let flow = handle_skill_outcome(
        &mut skill_menu,
        key(KeyCode::Right),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        skill_menu.is_some(),
        "the toggle list stays open after a flip"
    );
    assert!(
        marker_text(&chat).contains("[/skill] saved"),
        "expected a saved marker, got: {}",
        marker_text(&chat)
    );
    assert!(
        matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig { .. })),
        "a successful toggle must dispatch ReloadConfig"
    );

    // The toggle landed in the skills domain file (whose top level IS the
    // per-skill map — no `skills` wrapper), exactly one entry.
    let saved = read_json(&global_skills);
    assert_eq!(saved["alpha"]["enabled"], true);
    assert_eq!(
        saved.as_object().map(|o| o.len()),
        Some(1),
        "exactly one skills entry"
    );

    // The reloaded config reflects the toggle.
    assert_eq!(
        config.skills.get("alpha").map(|c| c.enabled),
        Some(true),
        "the handler must adopt the reloaded config"
    );

    // config.json is never created by a domain-only patch, and the project
    // domain file was never created either (the write went global).
    assert!(!tmp.path().join(".opencoder").join("config.json").exists());
    assert!(!workdir.join("opencoder.json").exists());
    assert!(!workdir.join(".opencoder").join("skills.json").exists());
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn skill_toggle_uses_the_active_env_domain_file() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let _iso = opencoder_core::scoped_config_home(home.clone());
    opencoder_core::create_config_env("alpha").unwrap();
    opencoder_core::activate_config_env("alpha").unwrap();
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();

    let mut config = Config {
        model: "session-only/shared".into(),
        ..Default::default()
    };
    config.providers.insert(
        "session-only".into(),
        opencoder_core::ProviderConfig {
            base_url: "https://example.invalid/session".into(),
            api_key: Some("placeholder".into()),
            ..Default::default()
        },
    );
    config.modes.plan = Some(opencoder_core::ModeConfig {
        model: Some("session-only/plan".into()),
        ..Default::default()
    });
    config.cli.insert(
        "session-cli".into(),
        opencoder_core::CliConfig {
            enabled: true,
            content: "preserve this domain".into(),
            ..Default::default()
        },
    );
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(8);
    let mut cmd_rx = auto_ack_commands(cmd_rx);
    let mut chat = ChatView {
        shared_model: config.model.clone(),
        model: "session-only/plan".into(),
        ..Default::default()
    };
    let mut menu = Some(SkillMenu::List(SkillList::from_discovered(
        &[discovered_skill()],
        &config,
    )));
    handle_skill_outcome(
        &mut menu,
        key(KeyCode::Right),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    let env_skills = home.join(".opencoder/envs/alpha/skills.json");
    assert_eq!(read_json(&env_skills)["alpha"]["enabled"], true);
    assert_eq!(
        config.skills.get("alpha").map(|skill| skill.enabled),
        Some(true)
    );
    assert_eq!(config.model, "session-only/shared");
    assert!(config.providers.contains_key("session-only"));
    assert_eq!(
        config
            .modes
            .plan
            .as_ref()
            .and_then(|mode| mode.model.as_deref()),
        Some("session-only/plan")
    );
    assert!(config.cli.contains_key("session-cli"));
    assert_eq!(chat.shared_model, "session-only/shared");
    assert_eq!(chat.model, "session-only/plan");
    let command = cmd_rx.recv().await.expect("Skill domain reload");
    match command {
        UiCmd::ReloadConfig {
            config: candidate,
            scope: crate::worker::ConfigReloadScope::Skills,
            ..
        } => {
            assert_eq!(candidate.model, "session-only/shared");
            assert!(candidate.providers.contains_key("session-only"));
            assert!(candidate.cli.contains_key("session-cli"));
            assert_eq!(
                candidate.skills.get("alpha").map(|skill| skill.enabled),
                Some(true)
            );
        }
        other => panic!("unexpected reload command: {other:?}"),
    }
    assert!(!home.join(".opencoder/skills.json").exists());
    assert!(!workdir.join(".opencoder/skills.json").exists());
    assert!(!workdir.join("opencoder.json").exists());
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn skill_reload_dispatch_failure_keeps_live_config_and_reports_disk_save() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().to_path_buf());
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let mut config = Config::default();
    let config_before = serde_json::to_value(&config).unwrap();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(1);
    drop(cmd_rx);
    let mut chat = ChatView::default();
    let mut menu = Some(SkillMenu::List(SkillList::from_discovered(
        &[discovered_skill()],
        &config,
    )));

    handle_skill_outcome(
        &mut menu,
        key(KeyCode::Right),
        &mut config,
        &cmd_tx,
        &mut chat,
        &workdir,
    )
    .await;

    assert_eq!(serde_json::to_value(&config).unwrap(), config_before);
    assert!(menu.is_some());
    assert_eq!(
        read_json(&tmp.path().join(".opencoder/skills.json"))["alpha"]["enabled"],
        true
    );
    let text = marker_text(&chat);
    assert!(text.contains("reload dispatch failed"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn skill_worker_failure_keeps_outer_domain_and_reports_disk_save() {
    let _guard = HOME_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let _iso = opencoder_core::scoped_config_home(tmp.path().join("home"));
    let workdir = tmp.path().join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let mut config = Config::default();
    let before = serde_json::to_value(&config).unwrap();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(4);
    let mut chat = ChatView::default();
    let mut menu = Some(SkillMenu::List(SkillList::from_discovered(
        &[discovered_skill()],
        &config,
    )));

    let (_, command) = tokio::join!(
        handle_skill_outcome(
            &mut menu,
            key(KeyCode::Right),
            &mut config,
            &cmd_tx,
            &mut chat,
            &workdir,
        ),
        async {
            let command = cmd_rx.recv().await.expect("Skill reload command");
            match &command {
                UiCmd::ReloadConfig { ack: Some(ack), .. } => {
                    ack.send(Err("injected worker failure".into())).unwrap();
                }
                other => panic!("unexpected command: {other:?}"),
            }
            command
        }
    );

    assert!(matches!(
        command,
        UiCmd::ReloadConfig {
            scope: crate::worker::ConfigReloadScope::Skills,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(&config).unwrap(), before);
    assert!(tmp.path().join("home/.opencoder/skills.json").exists());
    let text = marker_text(&chat);
    assert!(text.contains("runtime reload failed"), "{text}");
    assert!(text.contains("disk changes were saved"), "{text}");
    assert!(text.contains("live config was not refreshed"), "{text}");
}
