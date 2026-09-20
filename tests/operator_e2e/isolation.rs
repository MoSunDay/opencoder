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

fn read_probe(fleet: &Fleet, file: &str) -> String {
    let path = fleet
        .node_data
        .join("operator")
        .join(SESSION)
        .join("workspace")
        .join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read probe {file} at {}: {e}", path.display()))
        .trim()
        .to_string()
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
    // TEMP DEBUG
    let (_ds, detail_d) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    eprintln!("DEBUG session detail: {detail_d}");
    let exec_dir = fleet.node_data.join("operator").join(SESSION);
    eprintln!(
        "DEBUG exec_dir listing: {:?}",
        std::fs::read_dir(&exec_dir).map(|rd| rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>())
    );
    eprintln!(
        "DEBUG workspace listing: {:?}",
        std::fs::read_dir(&workspace).map(|rd| rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>())
    );
    eprintln!("DEBUG tmp listing: {:?}", std::fs::read_dir(tmp.path()).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>()));
    eprintln!("DEBUG request count: {}", stub.request_count());
    let _ = stub.wait_for_requests(1);
    let reqs = stub.wait_for_requests(1);
    eprintln!("DEBUG first request (truncated): {}", &reqs.get(0).cloned().unwrap_or_default()[..reqs.get(0).map(|r| r.len().min(2000)).unwrap_or(0)]);
    let home = fleet.node_data.join("operator").join(SESSION).join("home");

    // cwd: the bash tool ran inside the per-execution workspace (probe
    // files exist there), NOT in the node workdir.
    assert_eq!(
        read_probe(&fleet, "pwd-a.txt"),
        workspace.to_string_lossy(),
        "pwd must be the execution workspace"
    );
    assert!(
        !tmp.path().join("pwd-a.txt").exists(),
        "probe leaked into the node workdir"
    );
    // HOME: the execution home, not the daemon process home (= fleet workdir).
    assert_eq!(
        read_probe(&fleet, "home-a.txt"),
        home.to_string_lossy(),
        "HOME must be the execution home"
    );
    assert_ne!(read_probe(&fleet, "home-a.txt"), tmp.path().to_string_lossy());
    // The frozen config snapshot is visible under the redirected HOME.
    assert_eq!(read_probe(&fleet, "cfg-a.txt"), "CONFIG_OK");

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
    assert!(transcript.contains("PROBE-A-DONE"), "transcript: {transcript}");

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
        read_probe(&fleet, "pwd-b.txt"),
        workspace.to_string_lossy(),
        "cwd after restart must still be the workspace"
    );
    assert_eq!(
        read_probe(&fleet, "home-b.txt"),
        home.to_string_lossy(),
        "HOME must be rebuilt from the persisted harness envs"
    );
    assert_eq!(read_probe(&fleet, "cfg-b.txt"), "CONFIG_OK");

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
