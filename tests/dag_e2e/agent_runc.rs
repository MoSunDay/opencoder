//! Shared DAG container end to end: a single agent step's
//! WHOLE session (LLM loop + tools) runs inside the read-only OCI container
//! via the rootfs-installed `agent-step-runner`, while the host keeps the
//! session row + artifacts contract. Pins: structured output recovered from
//! the container-written `output.json`, runner artifacts
//! (`transcript.txt`/`session.json`), host/container session-id agreement,
//! kernel-enforced READ-ONLY knowledge root (byte-identical tree, no write
//! probe), and the bundle's direct-argv + env + knowledge-mount shape.
//!
//! Skips (with a reason) when `runc` is not installed — the sandbox is a
//! provisioning concern, not a test prerequisite.

use std::collections::BTreeMap;
use std::path::Path;

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script};
use serde_json::{json, Value};

const DEF: &str = "e2e-agent-runc";
const RUN: &str = "dag-e2e-agent-runc-1";
const STEP: &str = "probe";

fn read_json(path: &Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// Read a step artifact for diagnostics; missing files surface as empty.
fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Recursive (rel-path → kind, len, mtime-nanos) snapshot of a tree: the
/// knowledge root must be byte-identical after a sandboxed step ran against
/// it (the ro bind is kernel-enforced; this pins it at the artifact level).
fn snapshot_tree(root: &Path) -> BTreeMap<String, (String, u64, u128)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        {
            let entry = entry.expect("knowledge entry");
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .expect("entry under root")
                .to_string_lossy()
                .into_owned();
            let meta = std::fs::symlink_metadata(&path).expect("knowledge metadata");
            let kind = if meta.is_dir() {
                "dir"
            } else if meta.is_symlink() {
                "symlink"
            } else {
                "file"
            };
            let mtime = meta
                .modified()
                .expect("mtime")
                .duration_since(std::time::UNIX_EPOCH)
                .expect("mtime after epoch")
                .as_nanos();
            out.insert(rel, (kind.to_string(), meta.len(), mtime));
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    out
}

fn runc_available() -> bool {
    std::process::Command::new("runc")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn agent_step_session_runs_inside_runc_container() {
    if !runc_available() {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    // Any request (the step turn) gets the fenced structured-output reply.
    let stub = LlmStub::spawn(vec![Script::dynamic(|_req| {
        "分析完成。\n```json\n{\"verdict\": \"ok\"}\n```\n".to_string()
    })]);
    let tmp = tempfile::tempdir().unwrap();

    // Knowledge tree: one nested file the step prompt references.
    let knowledge = tmp.path().join("knowledge");
    std::fs::create_dir_all(knowledge.join("docs")).unwrap();
    std::fs::write(
        knowledge.join("docs/guide.md"),
        "# 指南\n只读知识库样例。\n",
    )
    .unwrap();
    let knowledge_abs = std::fs::canonicalize(&knowledge).unwrap();
    let before = snapshot_tree(&knowledge_abs);

    let fleet = Fleet::spawn_native(
        tmp.path(),
        stub.port(),
        json!({"dag": { "knowledge_root": knowledge_abs}}),
        "agent-runc-node",
    );

    let spec = json!({
        "name": DEF,
        "steps": [
            {"name": STEP,
             "kind": {"type": "agent", "prompt": "读取知识库 docs/guide.md 并给出结论"}},
        ],
    });
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec}));
    assert_eq!(status, 200, "save def: {body}");
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{DEF}/dispatch"),
        &json!({"id": RUN}),
    );
    assert_eq!(status, 202, "dispatch {RUN}: {body}");

    let doc = fleet.wait_terminal(RUN);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");

    let step_dir = fleet.run_root(RUN).join(STEP);
    // Structured output recovered from the container-side runner's reply.
    let output = read_json(&step_dir.join("output.json"));
    assert_eq!(output["verdict"], json!("ok"), "output.json: {output}");
    // Runner artifacts: non-empty transcript + terminal session.json.
    let transcript = read_text(&step_dir.join("transcript.txt"));
    assert!(
        !transcript.trim().is_empty(),
        "transcript.txt empty; step output:\n{}",
        read_text(&step_dir.join("output.txt"))
    );
    let session = read_json(&step_dir.join("session.json"));
    let session_id = session["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!session_id.is_empty(), "session.json: {session}");
    assert_eq!(session["status"], json!("done"), "session.json: {session}");
    // Host meta.json agrees with the container session id.
    let meta = read_json(&step_dir.join("meta.json"));
    assert_eq!(meta["outcome"], json!("done"), "meta.json: {meta}");
    assert_eq!(
        meta["session_id"].as_str(),
        Some(session_id.as_str()),
        "meta.json session_id must match the container session: {meta}"
    );

    // Zero writes into the knowledge root: identical snapshot, no probe file.
    let after = snapshot_tree(&knowledge_abs);
    assert_eq!(before, after, "knowledge root must stay byte-identical");
    assert!(
        !knowledge_abs.join(".ro-probe").exists(),
        "a write into the read-only knowledge root succeeded"
    );

    // Bundle shape: direct argv, injected LLM env, ro knowledge mount.
    let bundle = read_json(&fleet.run_root(RUN).join("bundle/config.json"));
    assert_eq!(
        bundle["process"]["args"],
        json!(["/usr/bin/dag-runner", "init"]),
        "bundle process.args: {bundle}"
    );
    let env = bundle["process"]["env"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !env.iter()
            .any(|entry| entry.as_str().unwrap_or_default().contains("API_KEY=")),
        "{env:?}"
    );
    let mounts = bundle["mounts"].as_array().cloned().unwrap_or_default();
    let knowledge_mount = mounts
        .iter()
        .find(|m| m["destination"] == json!("/run/opencoder/knowledge"))
        .unwrap_or_else(|| panic!("no knowledge mount in bundle: {mounts:?}"));
    let options = knowledge_mount["options"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        options.contains(&json!("ro")),
        "knowledge mount must be read-only: {knowledge_mount}"
    );
}

#[test]
fn event_stream_failure_is_resumable_and_retry_preserves_old_streams() {
    if !runc_available() {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    let stub = LlmStub::spawn(vec![
        Script::Hold,
        Script::Text("Recovered.\n```json\n{\"verdict\":\"recovered\"}\n```".into()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_native(tmp.path(), stub.port(), json!({}), "event-retry-node");
    let definition = "e2e-event-retry";
    let run = "dag-e2e-event-retry";
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec": {
            "name": definition,
            "steps": [{"name": STEP, "kind": {"type": "agent", "prompt": "Return a verdict"}}]
        }}),
    );
    assert_eq!(status, 200, "save: {body}");
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{definition}/dispatch"),
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch: {body}");
    stub.wait_until_entered();
    let root = fleet.run_root(run);
    let guest_step = root.join("workspace").join(STEP);
    let first_session = read_json(&guest_step.join("session.json"));
    let first_id = first_session["session_id"].as_str().unwrap();
    let first_events = guest_step
        .join("agent-events")
        .join(format!("{first_id}.ndjson"));
    // Inject an actual stream I/O contract failure while the container is alive.
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&first_events)
        .unwrap()
        .write_all(b"invalid-event\n")
        .unwrap();
    let failed = fleet.wait_terminal(run);
    stub.release();
    assert_eq!(
        failed["execution"]["status"], "error",
        "internal failure must not cancel the run: {failed}"
    );
    let meta = read_json(&root.join(STEP).join("meta.json"));
    assert!(
        meta["error"]
            .as_str()
            .unwrap()
            .contains("invalid container event"),
        "{meta}"
    );
    // The merged workspace is unmounted at terminal state; inspect retained upper files.
    let guest_step = root.join("upper").join(STEP);
    let first_events = guest_step
        .join("agent-events")
        .join(format!("{first_id}.ndjson"));
    let old_bytes = std::fs::read(&first_events).unwrap();
    let legacy = guest_step.join("events.ndjson");
    std::fs::write(&legacy, b"old-attempt-truncated-event\n").unwrap();
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/executions/{run}/commands"),
        &json!({"action":"resume", "input":{}}),
    );
    assert_eq!(status, 200, "resume after event error: {body}");
    let done = fleet.wait_terminal(run);
    assert_eq!(done["execution"]["status"], "done", "retry: {done}");
    let session = read_json(&guest_step.join("session.json"));
    let new_id = session["session_id"].as_str().unwrap();
    assert_ne!(first_id, new_id);
    assert_eq!(std::fs::read(&first_events).unwrap(), old_bytes);
    assert_eq!(
        std::fs::read(&legacy).unwrap(),
        b"old-attempt-truncated-event\n"
    );
    let events = read_text(
        &guest_step
            .join("agent-events")
            .join(format!("{new_id}.ndjson")),
    );
    assert!(!events.is_empty());
    for line in events.lines() {
        serde_json::from_str::<Value>(line).expect("new session events are complete JSON");
    }
    assert_eq!(
        read_json(&root.join(STEP).join("output.json"))["verdict"],
        "recovered"
    );
}
