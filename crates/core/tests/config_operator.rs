//! Operator-plane configuration source: `Config::load_operator` reads ONLY
//! the operator config directory — never the TUI project candidates, the
//! interactive user's real `~/.opencoder`, or env overlays.

use opencoder_core::{config::scoped_config_home, Config};

#[test]
fn load_operator_reads_only_the_operator_directory() {
    let project = tempfile::tempdir().unwrap();
    let interactive = tempfile::tempdir().unwrap(); // the real (scoped) home
    let operator = tempfile::tempdir().unwrap();

    let _guard = scoped_config_home(interactive.path().to_path_buf());
    // The interactive TUI side: project config + real global home + mcp.json.
    std::fs::write(
        project.path().join("opencoder.json"),
        serde_json::json!({"model": "tui/from-project", "fps": 12}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(interactive.path().join(".opencoder")).unwrap();
    std::fs::write(
        interactive.path().join(".opencoder/config.json"),
        serde_json::json!({"model": "tui/from-home"}).to_string(),
    )
    .unwrap();
    std::fs::write(
        interactive.path().join(".opencoder/mcp.json"),
        serde_json::json!({"tui-mcp": {"command": "tui"}}).to_string(),
    )
    .unwrap();

    // The operator plane carries its own complete view.
    std::fs::write(
        operator.path().join("config.json"),
        serde_json::json!({"model": "operator/own-model"}).to_string(),
    )
    .unwrap();
    std::fs::write(
        operator.path().join("mcp.json"),
        serde_json::json!({"operator-mcp": {"command": "operator"}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        operator.path().join("skills.json"),
        serde_json::json!({"operator-skill": {"enabled": true}}).to_string(),
    )
    .unwrap();

    // Live discovery still works exactly as the TUI expects.
    let live = Config::load(project.path()).unwrap();
    assert_eq!(live.model, "tui/from-project");
    assert_eq!(live.fps, Some(12));
    assert!(live.mcp_servers.contains_key("tui-mcp"));

    // The operator view is bound to its own directory: no project
    // candidate, no interactive home, no env overlay can reach it.
    let cfg = Config::load_operator(operator.path()).unwrap();
    assert_eq!(cfg.model, "operator/own-model");
    assert!(cfg.fps.is_none(), "project candidate must not leak");
    assert!(
        cfg.mcp_servers.contains_key("operator-mcp"),
        "operator domain files load from the operator dir: {:?}",
        cfg.mcp_servers
    );
    assert!(
        !cfg.mcp_servers.contains_key("tui-mcp"),
        "interactive home mcp.json must not leak"
    );
    assert_eq!(
        cfg.skills.get("operator-skill").map(|s| s.enabled),
        Some(true)
    );
}
