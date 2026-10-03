//! Validate the optional computer CLI example against OpenCoder's actual loader.

use opencoder_core::{AgentMode, Config};

#[test]
fn computer_cli_example_loads_and_injects_into_primary_agent() {
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let directory = project.path().join(".opencoder");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("cli.json"),
        include_str!("../../../tools/computer-use/examples/cli.json"),
    )
    .unwrap();

    let config = Config::load_with_home_frozen(project.path(), Some(home.path())).unwrap();
    let entries = config.enabled_cli_for("act", AgentMode::Primary);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "computer-use");
    for command in ["doctor", "run", "status", "cancel"] {
        assert!(entries[0].1.content.contains(command));
    }
    assert!(config
        .enabled_cli_for("explore", AgentMode::Subagent)
        .is_empty());
}

#[test]
fn disabled_computer_cli_is_not_injected() {
    let mut config = Config {
        cli: serde_json::from_str(include_str!(
            "../../../tools/computer-use/examples/cli.json"
        ))
        .unwrap(),
        ..Config::default()
    };
    config.cli.get_mut("computer-use").unwrap().enabled = false;
    assert!(config.enabled_cli_for("act", AgentMode::Primary).is_empty());
}
