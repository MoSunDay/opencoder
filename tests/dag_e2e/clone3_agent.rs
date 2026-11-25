//! Managed-harness agent steps inside the native sandbox: an agent card
//! pinned to the `codex` harness runs a harness binary the runners image
//! provides (a fake speaking the JSONL protocol, reached through the
//! read-only `/run/opencoder/bin` bind) inside the sandbox, with its
//! credential home bound read-write, the host mapping still read-only and
//! the receipts archived on the host; a runners image without the harness
//! is refused in preflight.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::{json, Value};

use crate::clone3_fixtures::{
    assert_released, backend_ready, expected_backends, read_json, recorded,
};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use crate::support::native::copy_runners_image;

const DEF: &str = "e2e-codex-sandbox";
const RUN: &str = "dag-e2e-codex-sandbox-1";
const STEP: &str = "probe";
/// Guest path of the node's runners directory (the read-only bind of the
/// image's `usr/bin`): where the sandbox reaches the harness binary.
const RUNNERS_MOUNT: &str = "/run/opencoder/bin";
const HARNESS: &str = "/run/opencoder/bin/codex";

/// The harness binary of the runners image copy: records its argv, checks
/// the sandbox pins it can see and answers with one completed turn.
const FAKE_HARNESS: &str = r##"#!/bin/sh
set -eu
prompt="$(cat)"
printf '%s\n' "$@" > "$OPENCODER_STEP_DIR/argv.txt"
[ "$(cat "$CODEX_HOME/auth.json")" = fixture-login ]
[ "$(cat /proc/sys/kernel/hostname)" = dag-run ]
if (printf forbidden > /etc/.write-probe) 2>/dev/null; then exit 31; fi
if (printf forbidden > /usr/bin/.write-probe) 2>/dev/null; then exit 31; fi
printf refreshed > "$CODEX_HOME/refresh-observed"
printf '%s\n' '{"type":"thread.started","thread_id":"e2e-codex-thread"}' '{"type":"turn.started"}'
printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","id":"answer","text":"```json\n{\"ok\":true}\n```"}}'
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":8,"output_tokens":4}}'
"##;

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Agent card of the harness, in the server-side source of the read-only
/// agents export, plus the prompt it references.
fn write_agent_card(agents: &Path) {
    write(
        &agents.join("codex-e2e/meta.json"),
        &json!({"name": "codex-e2e", "harness": "codex", "current": {"prompt": "probe"}})
            .to_string(),
    );
    write(
        &agents.join("prompts/probe/meta.json"),
        &json!({"name": "probe", "current": 1, "history": [1]}).to_string(),
    );
    write(
        &agents.join("prompts/probe/v1/soul.md"),
        "Sandbox harness probe\n",
    );
}

#[test]
fn harness_agent_step_runs_the_runners_image_binary_inside_the_sandbox() {
    if !backend_ready() {
        return;
    }
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    write_agent_card(&tmp.path().join("native-source/agents"));
    // Credential home of the harness: bound read-write into the sandbox.
    let codex_home = tmp.path().join(".codex");
    std::fs::create_dir_all(&codex_home).unwrap();
    std::fs::write(codex_home.join("auth.json"), "fixture-login").unwrap();
    // A per-test copy of the runners image carrying the harness binary next
    // to the runners (the shared image has none): its `usr/bin` is the
    // node's runners directory, so the sandbox reaches the harness through
    // the read-only `/run/opencoder/bin` bind.
    let image = tmp.path().join("codex-runners-image");
    copy_runners_image(&image);
    let harness = image.join("usr/bin/codex");
    write(&harness, FAKE_HARNESS);
    std::fs::set_permissions(&harness, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fleet = Fleet::spawn_native_with(
        tmp.path(),
        stub.port(),
        json!({}),
        "codex-sandbox-node",
        vec![("CODEX_HOME".into(), codex_home.display().to_string())],
    );
    // The fixture pins `dag.rootfs_dir` to the shared image: point the node
    // at the copy instead and pin the harness profile to the copy's binary
    // (read per dispatch).
    let config_file = fleet.workdir.join("node-work/.opencoder/config.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&config_file).unwrap()).unwrap();
    config["dag"]["rootfs_dir"] = json!(image);
    config["agent"]["codex"] = json!({"executable": HARNESS});
    std::fs::write(&config_file, config.to_string()).unwrap();

    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec": {"name": DEF, "steps": [
            {"name": STEP, "kind": {"type": "agent", "agent": "codex-e2e", "prompt": "probe the sandbox"}},
        ]}}),
    );
    assert_eq!(status, 200, "save def: {body}");
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{DEF}/dispatch"),
        &json!({"id": RUN}),
    );
    // The profile names the harness by its guest path: preflight must resolve
    // it inside the node's runners directory (`dag.rootfs_dir/usr/bin`), not
    // as a host path.
    assert_eq!(
        status,
        202,
        "dispatch {RUN}: {body} — a profile executable under {RUNNERS_MOUNT} must resolve inside the node's runners directory"
    );
    let doc = fleet.wait_terminal(RUN);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");
    assert_eq!(
        stub.request_count(),
        0,
        "a harness step never calls the LLM"
    );

    // Receipts archived on the host from the sandboxed harness run.
    let run_root = fleet.run_root(RUN);
    let step_dir = run_root.join(STEP);
    assert_eq!(
        read_json(&step_dir.join("output.json")),
        json!({"ok": true})
    );
    let session = read_json(&step_dir.join("session.json"));
    assert_eq!(session["harness"], "codex", "{session}");
    assert_eq!(session["thread_id"], "e2e-codex-thread", "{session}");
    assert_eq!(session["status"], "done", "{session}");
    let meta = read_json(&step_dir.join("meta.json"));
    assert_eq!(meta["outcome"], "done", "{meta}");
    assert_eq!(meta["session_id"], session["session_id"], "{meta}");
    assert!(
        step_dir.join("messages.json").is_file(),
        "archived receipts"
    );
    assert!(
        step_dir.join("transcript.txt").is_file(),
        "archived receipts"
    );

    // The harness ran as the `exec` JSONL protocol driver from inside the
    // sandbox and could refresh its credential home, which is rw-bound.
    let argv: Vec<String> =
        std::fs::read_to_string(run_root.join("upper").join(STEP).join("argv.txt"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
    assert!(argv.iter().any(|arg| arg == "exec"), "{argv:?}");
    assert!(argv.iter().any(|arg| arg == "--json"), "{argv:?}");
    assert_eq!(argv.last().unwrap(), "-", "{argv:?}");
    assert_eq!(
        std::fs::read_to_string(codex_home.join("refresh-observed")).unwrap(),
        "refreshed"
    );

    // Backend receipt and the sandbox shape the harness ran in: the init is
    // the runners' `dag-runner` behind a sealed root, and the harness is
    // reachable only through the read-only bind of the runners image.
    let receipt = read_json(&run_root.join("container.json"));
    assert!(
        expected_backends().contains(&receipt["backend"].as_str().unwrap_or_default()),
        "{receipt}"
    );
    let bundle = read_json(&run_root.join("bundle/config.json"));
    assert_eq!(
        bundle["process"]["args"],
        json!(["/run/opencoder/bin/dag-runner", "init"])
    );
    assert_eq!(bundle["root"]["path"], json!("root"));
    assert_eq!(bundle["root"]["readonly"], json!(true));
    assert_eq!(
        bundle["mounts"][0]["destination"],
        json!("/"),
        "the first OCI mount is the tmpfs root: {bundle}"
    );
    assert_eq!(bundle["mounts"][0]["type"], json!("tmpfs"));
    let mounts = bundle["mounts"].as_array().cloned().unwrap_or_default();
    let runners = mounts
        .iter()
        .find(|mount| mount["destination"] == json!(RUNNERS_MOUNT))
        .unwrap_or_else(|| panic!("no runners mount in {mounts:?}"));
    assert_eq!(
        std::fs::canonicalize(runners["source"].as_str().unwrap()).unwrap(),
        harness.parent().unwrap().canonicalize().unwrap(),
        "the runners bind is not the harness image: {runners}"
    );
    assert!(runners["options"]
        .as_array()
        .unwrap()
        .contains(&json!("ro")));
    let env = bundle["process"]["env"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !env.iter()
            .any(|entry| entry.as_str().unwrap_or_default().contains("API_KEY=")),
        "{env:?}"
    );
    let home_mount = mounts
        .iter()
        .find(|mount| {
            std::fs::canonicalize(mount["source"].as_str().unwrap_or("")).ok()
                == codex_home.canonicalize().ok()
        })
        .unwrap_or_else(|| panic!("no credential home mount in {mounts:?}"));
    assert!(home_mount["options"]
        .as_array()
        .unwrap()
        .contains(&json!("rw")));
    assert!(!home_mount["options"]
        .as_array()
        .unwrap()
        .contains(&json!("ro")));
    let private = mounts
        .iter()
        .find(|mount| mount["destination"] == json!("/run/opencoder-private"))
        .unwrap_or_else(|| panic!("no private mount in {mounts:?}"));
    assert!(private["options"]
        .as_array()
        .unwrap()
        .contains(&json!("ro")));
    assert_released(&run_root, recorded(&run_root).as_ref(), fleet.agent_pid());

    // A runners image without the harness binary is refused in preflight,
    // naming the guest path the sandbox would have reached it through.
    std::fs::remove_file(&harness).unwrap();
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{DEF}/dispatch"),
        &json!({"id": "dag-e2e-codex-sandbox-2"}),
    );
    assert_eq!(status, 400, "dispatch without the harness: {body}");
    let error = body["error"].as_str().unwrap_or_default();
    assert!(error.contains("preflight"), "{error}");
    assert!(
        error.contains(&format!(
            "Codex executable missing on the host at {HARNESS}; install it under /usr or dag.runners_dir"
        )),
        "{error}"
    );
    assert_eq!(stub.request_count(), 0);
}
