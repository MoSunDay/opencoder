//! Fresh-session agent selection in the TUI bootstrap honors TuiOpts'
//! `--agent` passthrough and the effective-default chain, so file-based
//! custom agents become the session's primary agent. The bootstrap itself
//! enters the terminal, so the decision is covered at its pure seam
//! (`fresh_agent_name` + `resolve_agent` — exactly what app_bootstrap
//! calls), following the worker-level fixture conventions of the other
//! tui tests.

use std::sync::{Mutex, MutexGuard};

use opencoder_core::agent::set_agents_dir_override;
use opencoder_core::{resolve_agent, Config};
use opencoder_tui::{fresh_agent_name, TuiOpts};

/// Serializes tests touching the process-global agents-root override.
static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

/// Point the agents root at a fresh tempdir under the override lock. The
/// returned guard must be held for the whole test body.
fn scoped_agents() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}

/// Minimal resolvable file agent: a private prompt pool `prompts/<name>/v1`
/// (soul only) plus a card referencing it.
fn write_file_agent(root: &std::path::Path, name: &str, soul: &str) {
    let pool = root.join("prompts").join(name);
    let vdir = pool.join("v1");
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::write(vdir.join("soul.md"), soul).unwrap();
    std::fs::write(
        pool.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": 1, "history": [1] }}"#),
    )
    .unwrap();
    let adir = root.join(name);
    std::fs::create_dir_all(&adir).unwrap();
    std::fs::write(
        adir.join("meta.json"),
        format!(r#"{{ "name": "{name}", "current": {{ "prompt": "{name}" }} }}"#),
    )
    .unwrap();
}

fn opts(agent: Option<&str>) -> TuiOpts {
    TuiOpts::new(None).with_agent(agent.map(str::to_string))
}

fn cfg_with_default(name: &str) -> Config {
    let mut cfg = Config::default();
    cfg.agent.default = name.into();
    cfg
}

/// `TuiOpts { agent: Some("file-agent") }` names that agent for the fresh
/// session, and the chosen name resolves to the file agent's card (Act
/// kind, Primary mode, composed prompt body).
#[test]
fn agent_override_names_a_resolvable_file_agent() {
    let (dir, _g) = scoped_agents();
    write_file_agent(dir.path(), "writer", "Writer soul: small diffs.");

    // The override wins even when the config default names another agent.
    let name = fresh_agent_name(&opts(Some("writer")), &cfg_with_default("plan"));
    assert_eq!(name, "writer");

    // The bootstrap then does resolve_agent(name): it must land on the file
    // agent (what SessionState::new would receive as the primary agent).
    let agent = resolve_agent(&name).expect("file agent must resolve");
    assert_eq!(agent.name, "writer");
    assert!(agent.is_primary());
    assert_eq!(agent.kind, opencoder_core::AgentKind::Act);
    assert!(
        agent.prompt.contains("Writer soul"),
        "chosen agent must carry the card's prompt body"
    );
}

/// Without an override, the active file-agent marker wins over the config
/// default; with neither, the config default (then "act") decides.
#[test]
fn without_override_the_marker_then_config_default_decide() {
    let (dir, _g) = scoped_agents();
    write_file_agent(dir.path(), "writer", "Writer soul.");

    // Marker tier: active file agent beats a non-empty config default.
    std::fs::write(dir.path().join("active"), "writer").unwrap();
    assert_eq!(
        fresh_agent_name(&opts(None), &cfg_with_default("plan")),
        "writer"
    );
    // Explicit --agent still outranks the marker.
    assert_eq!(
        fresh_agent_name(&opts(Some("plan")), &cfg_with_default("plan")),
        "plan"
    );

    // Config-default tier once the marker is gone.
    std::fs::remove_file(dir.path().join("active")).unwrap();
    assert_eq!(
        fresh_agent_name(&opts(None), &cfg_with_default("plan")),
        "plan"
    );
    // Final tier: nothing set anywhere -> builtin default "act".
    assert_eq!(fresh_agent_name(&opts(None), &Config::default()), "act");
}
