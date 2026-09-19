//! M1 — `dag.agent_sandbox = "runc"` end to end: a single agent step's
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
use std::path::{Path, PathBuf};

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

    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {"agent_sandbox": "runc", "knowledge_root": knowledge_abs}}),
        "agent-runc-node",
    );

    // Rootfs provisioning (scaffold + wasmtime + agent-step-runner + libs)
    // through the repo script, into the node's shared-rootfs slot.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rootfs = fleet.node_data.join("dag").join("rootfs");
    let prep = std::process::Command::new("bash")
        .arg(repo.join("scripts").join("prepare-dag-rootfs.sh"))
        .arg(&rootfs)
        .output()
        .expect("spawn prepare-dag-rootfs.sh");
    assert!(
        prep.status.success(),
        "prepare-dag-rootfs.sh failed:\n-- stdout --\n{}\n-- stderr --\n{}",
        String::from_utf8_lossy(&prep.stdout),
        String::from_utf8_lossy(&prep.stderr),
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

    let step_dir = fleet.node_data.join("dag").join(RUN).join(STEP);
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
    let bundle = read_json(
        &fleet
            .node_data
            .join("dag")
            .join("bundles")
            .join(RUN)
            .join(STEP)
            .join("config.json"),
    );
    assert_eq!(
        bundle["process"]["args"],
        json!(["/usr/bin/agent-step-runner"]),
        "bundle process.args: {bundle}"
    );
    let env = bundle["process"]["env"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        env.iter()
            .any(|e| e.as_str().unwrap_or("").starts_with("OPENAI_BASE_URL=")),
        "bundle env must carry OPENAI_BASE_URL: {env:?}"
    );
    assert!(
        env.iter()
            .any(|e| e.as_str().unwrap_or("").starts_with("OPENAI_API_KEY=")),
        "bundle env must carry OPENAI_API_KEY: {env:?}"
    );
    let mounts = bundle["mounts"].as_array().cloned().unwrap_or_default();
    let knowledge_mount = mounts
        .iter()
        .find(|m| m["destination"] == json!("/workspace/knowledge"))
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
