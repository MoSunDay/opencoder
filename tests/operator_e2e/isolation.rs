//! O7 — per-execution HOME/WORKSPACE isolation for operator executions.
//!
//! A fresh operator execution materializes `<data>/operator/<id>/home` (a
//! frozen `config.json` snapshot, 0600) and `.../workspace`. The session's
//! bash tool runs with cwd = workspace and HOME = the execution home —
//! both verified from INSIDE a real turn via a scripted tool call, then
//! re-verified on-disk. After an agent restart, a follow-up prompt proves
//! the pair survives resume (harness envs → `env_passthrough`, and the
//! relay's config reload reads the frozen home, not the daemon's).

use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::llm_stub::{LlmStub, Script};
use serde_json::json;

const SESSION: &str = "operator-e2e-isolation-1";

/// One probe turn: a bash tool call that drops `pwd`/`$HOME`/config-marker
/// files into the cwd and echoes a transcript-visible marker.
fn probe_tool_call(tag: &str) -> Script {
    let command = format!(
        "pwd > pwd-{tag}.txt; \
         printf '%s\\n' \"$HOME\" > home-{tag}.txt; \
         if test -f \"$HOME/.opencoder/config.json\"; then echo CONFIG_OK > cfg-{tag}.txt; \
         else echo CONFIG_MISSING > cfg-{tag}.txt; fi; \
         echo PROBE-{tag}-DONE"
    );
    Script::ToolCall {
        name: "bash".into(),
        arguments: json!({ "command": command }).to_string(),
    }
}

fn read_probe(fleet: &Fleet, id: &str, file: &str) -> String {
    let path = fleet
        .node_data
        .join("operator")
        .join(id)
        .join("workspace")
        .join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read probe {file} at {}: {e}", path.display()))
        .trim()
        .to_string()
}

/// One probe turn listing the execution skill pool from inside the session.
fn skills_probe(tag: &str) -> Script {
    let command = format!(
        "ls \"$HOME/.opencoder/skills\" > skills-{tag}.txt; \
         echo FROZEN-{tag}-DONE"
    );
    Script::ToolCall {
        name: "bash".into(),
        arguments: json!({ "command": command }).to_string(),
    }
}

#[test]
fn operator_execution_isolates_home_and_workspace() {
    // Turn 1: tool call + closing text. Turn 2 (post-restart): same shape.
    let stub = LlmStub::spawn(vec![
        probe_tool_call("a"),
        Script::Text("isolation-turn-1-done".into()),
        probe_tool_call("b"),
        Script::Text("isolation-turn-2-done".into()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    let mut fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-iso-node");

    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": SESSION, "node_id": fleet.node_id(), "agent": "act", "prompt": "probe env"}),
    );
    assert_eq!(status, 200, "create: {body}");
    fleet.wait_idle(SESSION);

    let workspace = fleet
        .node_data
        .join("operator")
        .join(SESSION)
        .join("workspace");
    let home = fleet.node_data.join("operator").join(SESSION).join("home");

    // cwd: the bash tool ran inside the per-execution workspace (probe
    // files exist there), NOT in the node workdir.
    assert_eq!(
        read_probe(&fleet, SESSION, "pwd-a.txt"),
        workspace.to_string_lossy(),
        "pwd must be the execution workspace"
    );
    assert!(
        !tmp.path().join("pwd-a.txt").exists(),
        "probe leaked into the node workdir"
    );
    // HOME: the execution home, not the daemon process home (= fleet workdir).
    assert_eq!(
        read_probe(&fleet, SESSION, "home-a.txt"),
        home.to_string_lossy(),
        "HOME must be the execution home"
    );
    assert_ne!(
        read_probe(&fleet, SESSION, "home-a.txt"),
        tmp.path().to_string_lossy()
    );
    // The frozen config snapshot is visible under the redirected HOME.
    assert_eq!(read_probe(&fleet, SESSION, "cfg-a.txt"), "CONFIG_OK");

    // Snapshot contract: 0600, plaintext api key preserved (the execution's
    // client is built from this file on every reload).
    let snapshot = home.join(".opencoder/config.json");
    let text = std::fs::read_to_string(&snapshot).unwrap();
    assert!(text.contains("test-key"), "api key in snapshot: {text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&snapshot).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "snapshot must be owner-only");
    }

    // Turn 1's tool output is transcript-visible.
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    assert_eq!(status, 200);
    let transcript = detail["messages"].to_string();
    assert!(
        transcript.contains("PROBE-a-DONE"),
        "transcript: {transcript}"
    );

    // ── restart: the agent process comes back with the node daemon's own
    // HOME (= fleet workdir); the execution must rebuild its isolated pair.
    fleet.respawn_agent();
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{SESSION}/prompt"),
        &json!({"prompt": "probe env again", "input_id": "e2e-iso-followup"}),
    );
    assert_eq!(status, 200, "follow-up: {body}");
    fleet.wait_idle(SESSION);

    assert_eq!(
        read_probe(&fleet, SESSION, "pwd-b.txt"),
        workspace.to_string_lossy(),
        "cwd after restart must still be the workspace"
    );
    assert_eq!(
        read_probe(&fleet, SESSION, "home-b.txt"),
        home.to_string_lossy(),
        "HOME must be rebuilt from the persisted harness envs"
    );
    assert_eq!(read_probe(&fleet, SESSION, "cfg-b.txt"), "CONFIG_OK");

    // Both turns hit the LLM through the frozen config's provider endpoint:
    // 2 model calls per turn (tool call + closing reply) = 4 total.
    let requests = stub.wait_for_requests(4);
    assert!(
        requests.iter().any(|r| r.contains("probe env again")),
        "follow-up prompt reached the LLM"
    );

    // Sanity: the SSE stream still terminates for the relayed follow-up.
    let frames = crate::support::http_util::sse_read(
        &fleet.base,
        &format!("/api/sessions/{SESSION}/events"),
        TOKEN,
        0,
        None,
    );
    assert_eq!(frames.last().unwrap().event, "stream_end");
}

const FROZEN: &str = "operator-e2e-isolation-frozen";

/// O7b — the operator config plane stays frozen against interactive-side
/// edits, and the execution skill pool never sources the user's pool.
///
/// The first operator execution bootstraps `<data>/operator-config/` once
/// (config + the five domain files). Afterwards a TUI-style save rewrites
/// the node workdir `config.json` and a new pack lands in the global skill
/// pool (the node daemon's HOME = the fleet workdir). A brand-new operator
/// execution must still see the frozen plane: neither the mutated config
/// nor the pool packs reach `<data>/operator-config/`, the execution
/// snapshot, or the in-turn skill listing — builtins are seeded instead.
#[test]
fn operator_config_plane_frozen_against_workdir_and_user_pool() {
    let stub = LlmStub::spawn(vec![
        skills_probe("a"),
        Script::Text("frozen-first-done".into()),
        skills_probe("b"),
        Script::Text("frozen-second-done".into()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    // The node daemon runs with HOME = fleet workdir, so this directory is
    // simultaneously the interactive global skill pool and the workdir
    // skills domain.
    let pool = tmp.path().join(".opencoder/skills");
    std::fs::create_dir_all(&pool).unwrap();
    std::fs::write(pool.join("user-global.md"), "# user global\n").unwrap();

    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-iso-frozen");

    // First execution bootstraps the shared plane exactly once.
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": FROZEN, "node_id": fleet.node_id(), "agent": "act", "prompt": "frozen probe"}),
    );
    assert_eq!(status, 200, "create: {body}");
    fleet.wait_idle(FROZEN);

    let home = |id: &str| fleet.node_data.join("operator").join(id).join("home");
    let skills_a = home(FROZEN).join(".opencoder/skills");
    assert!(
        !skills_a.join("user-global.md").exists(),
        "interactive pool leaked into the execution home"
    );
    assert!(
        skills_a.join("task-plan/SKILL.md").is_file(),
        "builtins must be seeded into the execution home"
    );

    // Interactive-side edits AFTER the plane exists: a TUI-style save into
    // the shared workdir config plus a fresh pack in the global pool.
    let cfg_path = tmp.path().join(".opencoder/config.json");
    let mut cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    cfg["e2e_mutated"] = json!(true);
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
    std::fs::write(pool.join("user-late.md"), "# user late\n").unwrap();

    // A brand-new operator execution must not follow.
    let second = "operator-e2e-isolation-frozen-2";
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": second, "node_id": fleet.node_id(), "agent": "act", "prompt": "frozen again"}),
    );
    assert_eq!(status, 200, "create second: {body}");
    fleet.wait_idle(second);

    // The plane itself stayed frozen...
    let plane_cfg =
        std::fs::read_to_string(fleet.node_data.join("operator-config/config.json")).unwrap();
    assert!(
        !plane_cfg.contains("e2e_mutated"),
        "plane followed the TUI save: {plane_cfg}"
    );
    // ...and so did the fresh execution's snapshot.
    let snap = std::fs::read_to_string(home(second).join(".opencoder/config.json")).unwrap();
    assert!(snap.contains("test-key"), "snapshot: {snap}");
    assert!(!snap.contains("e2e_mutated"), "snapshot: {snap}");

    // Skill pool: frozen operator-plane packs + builtins only.
    let skills_b = home(second).join(".opencoder/skills");
    assert!(skills_b.join("task-plan/SKILL.md").is_file());
    assert!(!skills_b.join("user-global.md").exists());
    assert!(
        !skills_b.join("user-late.md").exists(),
        "pool drops after bootstrap must not leak"
    );
    // Same picture from inside a real turn.
    let listing = read_probe(&fleet, second, "skills-b.txt");
    assert!(listing.contains("task-plan"), "listing: {listing}");
    assert!(!listing.contains("user-global"), "listing: {listing}");
    assert!(!listing.contains("user-late"), "listing: {listing}");

    // Transcript shows the probe ran, and both turns reached the LLM through
    // the frozen config's provider endpoint (2 model calls per turn).
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{second}"), &json!({}));
    assert_eq!(status, 200);
    assert!(
        detail["messages"].to_string().contains("FROZEN-b-DONE"),
        "transcript: {}",
        detail["messages"]
    );
    let requests = stub.wait_for_requests(4);
    assert!(requests.iter().any(|r| r.contains("frozen again")));
}
