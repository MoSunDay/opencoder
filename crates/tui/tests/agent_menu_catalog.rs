//! Custom-only `/agent` catalog, using isolated on-disk registration cards.

use opencoder_core::agent::set_agents_dir_override;
use opencoder_tui::agent_menu::{available_primary_agents, AgentCard};

static OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct AgentsFixture {
    dir: tempfile::TempDir,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl AgentsFixture {
    fn new() -> Self {
        let lock = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        set_agents_dir_override(Some(dir.path().to_path_buf()));
        Self { dir, _lock: lock }
    }

    fn write_agent(&self, name: &str, soul: Option<&str>) {
        if let Some(soul) = soul {
            let pool = self.dir.path().join("prompts").join(name);
            let version = pool.join("v1");
            std::fs::create_dir_all(&version).unwrap();
            std::fs::write(version.join("soul.md"), soul).unwrap();
            std::fs::write(
                pool.join("meta.json"),
                format!(r#"{{"name":"{name}","current":1,"history":[1]}}"#),
            )
            .unwrap();
        }
        let card = self.dir.path().join(name);
        std::fs::create_dir_all(&card).unwrap();
        std::fs::write(
            card.join("meta.json"),
            format!(r#"{{"name":"{name}","current":{{"prompt":"{name}"}}}}"#),
        )
        .unwrap();
    }
}

impl Drop for AgentsFixture {
    fn drop(&mut self) {
        set_agents_dir_override(None);
    }
}

#[test]
fn empty_registration_has_no_builtin_cards() {
    let _fixture = AgentsFixture::new();
    assert!(available_primary_agents().is_empty());
}

#[test]
fn custom_cards_keep_sorted_names_and_existing_descriptions() {
    let fixture = AgentsFixture::new();
    fixture.write_agent("writer", Some("Writer soul: small diffs.\nmore"));
    fixture.write_agent("bare", None);
    assert_eq!(
        available_primary_agents(),
        vec![
            AgentCard {
                name: "bare".into(),
                description: "Custom agent bare".into(),
            },
            AgentCard {
                name: "writer".into(),
                description: "Writer soul: small diffs.".into(),
            },
        ]
    );
}

#[test]
fn builtin_named_directories_never_enter_the_catalog() {
    let fixture = AgentsFixture::new();
    for builtin in opencoder_core::builtin_agents() {
        fixture.write_agent(&builtin.name, Some("A same-named file card."));
    }
    assert!(available_primary_agents().is_empty());

    fixture.write_agent("writer", Some("Writer soul."));
    assert_eq!(
        available_primary_agents(),
        vec![AgentCard {
            name: "writer".into(),
            description: "Writer soul.".into(),
        }]
    );
}
