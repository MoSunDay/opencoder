//! Tests for `handle_skill_outcome` — the `/skill` modal's persistence layer.
//! The toggle patch (`{"skills":{<name>:{"enabled":<bool>}}}`) must land in
//! the `skills.json` domain file (global under the scoped home when no file
//! pre-exists), and `config.json` must not gain a `skills` key.

use std::path::PathBuf;

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
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCmd>(64);
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
        matches!(cmd_rx.recv().await, Some(UiCmd::ReloadConfig(_))),
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
