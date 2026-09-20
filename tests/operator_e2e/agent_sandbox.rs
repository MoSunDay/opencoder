//! O6 — the `run_mode` dispatch face of custom agent cards: a card pinned
//! `run_mode: "agent"` moves every `kind=agent` turn into a read-only runc
//! container (pinned pool bound at `/workspace/agent`, the rootfs-installed
//! `/usr/bin/agent-session-runner` as direct argv) while `run_mode:
//! "operator"` keeps the O5 host loop. Covered:
//! - fail-closed admission: no usable sandbox runtime (runc missing or no
//!   provisioned rootfs) rejects the create outright — no execution, no
//!   session, no model call;
//! - the sandbox round contract under runc: session-shaped result/output,
//!   host-side meta/messages/SSE surfaces, in-container pool resolution,
//!   OCI bundle shape, runner artifacts, and a follow-up prompt re-launching
//!   the workload so a NEW round continues the session (skips without runc).

use crate::support::fleet_proc::{Fleet, TOKEN};
use crate::support::http_util::{sse_read, SseFrame};
use crate::support::llm_stub::{LlmStub, Script};
use serde_json::{json, Value};
use std::path::Path;

/// Marker planted in the pinned pool's `soul.md`: it must reach the model
/// request composed INSIDE the container through `/workspace/agent`.
const SOUL_MARKER: &str = "e2e-sandbox-soul-marker-9f2a";
/// Marker for the operator-card pool (host-path resolution).
const HOST_SOUL_MARKER: &str = "e2e-host-soul-marker-3c1d";
const PROMPT1: &str = "e2e-sandbox-prompt-1: 沙箱会话第一轮";
const PROMPT2: &str = "e2e-sandbox-prompt-2: 沙箱会话第二轮";
const REPLY1: &str = "e2e-sandbox-reply-1";
const REPLY2: &str = "e2e-sandbox-reply-2";
const HOST_PROMPT: &str = "e2e-host-card-prompt: 主机模式回归";
const HOST_REPLY: &str = "e2e-host-card-reply";
const PREFLIGHT_SESSION: &str = "agent-e2e-sandbox-preflight-1";
const RUNC_SESSION: &str = "agent-e2e-sandbox-runc-1";
const HOST_SESSION: &str = "agent-e2e-sandbox-host-1";

/// Same availability probe as the dag_e2e runc suites.
fn runc_available() -> bool {
    std::process::Command::new("runc")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Provision the shared prompts pool (+ optional skills pool) and the
/// agent's reference card under `<workdir>/.opencoder/agents` BEFORE the
/// fleet spawns (HOME=<workdir> resolves the agents root; the node freezes
/// the pool into the pinned snapshot at admission).
fn write_card(
    workdir: &Path,
    agent: &str,
    pool: &str,
    run_mode: &str,
    soul: &str,
    skills: Option<(&str, &str)>,
) {
    let root = workdir.join(".opencoder").join("agents");
    let prompts = root.join("prompts").join(pool);
    std::fs::create_dir_all(prompts.join("v1")).unwrap();
    std::fs::write(prompts.join("meta.json"), r#"{"current":1}"#).unwrap();
    std::fs::write(prompts.join("v1").join("soul.md"), soul).unwrap();
    let mut current = json!({"prompt": pool});
    if let Some((skills_pool, skill)) = skills {
        let dir = root.join("skills").join(skills_pool);
        std::fs::create_dir_all(dir.join("v1").join(skill)).unwrap();
        std::fs::write(dir.join("meta.json"), r#"{"current":1}"#).unwrap();
        std::fs::write(
            dir.join("v1").join(skill).join("SKILL.md"),
            format!("# {skill}\nprobe skill for the sandbox e2e\n"),
        )
        .unwrap();
        current["skills"] = json!(skills_pool);
    }
    let card = root.join(agent);
    std::fs::create_dir_all(&card).unwrap();
    std::fs::write(
        card.join("meta.json"),
        json!({"name": agent, "run_mode": run_mode, "current": current}).to_string(),
    )
    .unwrap();
}

/// Provision `<node-data>/dag/rootfs` through the repo script (dag_e2e style).
fn prepare_rootfs(fleet: &Fleet) {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let prep = std::process::Command::new("bash")
        .arg(repo.join("scripts").join("prepare-dag-rootfs.sh"))
        .arg(fleet.node_data.join("dag").join("rootfs"))
        .output()
        .expect("spawn prepare-dag-rootfs.sh");
    assert!(
        prep.status.success(),
        "prepare-dag-rootfs.sh failed:\n-- stdout --\n{}\n-- stderr --\n{}",
        String::from_utf8_lossy(&prep.stdout),
        String::from_utf8_lossy(&prep.stderr),
    );
}

fn read_json(path: &Path) -> Value {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn contains_all(text: &str, needles: &[&str]) -> bool {
    needles.iter().all(|needle| text.contains(needle))
}

fn get(fleet: &Fleet, path: &str) -> (u16, Value) {
    fleet.http("GET", path, &json!({}))
}

fn out_text(doc: &Value) -> &str {
    doc["result"]["output_text"].as_str().unwrap_or_default()
}

/// Concatenated `text_delta` payloads of an SSE frame list.
fn text_deltas(frames: &[SseFrame]) -> String {
    frames
        .iter()
        .filter(|frame| frame.event == "text_delta")
        .map(|frame| frame.data["text"].as_str().unwrap_or_default())
        .collect()
}

/// Admission must fail closed: a `run_mode: agent` card without a usable
/// sandbox runtime rejects the create outright — the sandbox must never
/// silently fall back to the host session runtime.
#[test]
fn sandbox_agent_session_preflight_fails_closed() {
    let stub = LlmStub::spawn_text(&["never-reached"]);
    let tmp = tempfile::tempdir().unwrap();
    write_card(
        tmp.path(),
        "boxer",
        "boxer-pool",
        "agent",
        &format!("# boxer\n{SOUL_MARKER}\n"),
        None,
    );
    // No rootfs provisioned on purpose.
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-sbx-preflight");
    fleet.wait_ready(&["operator", "dag", "agent"]);

    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": PREFLIGHT_SESSION, "kind": "agent", "agent": "boxer",
                "node_id": fleet.node_id(), "prompt": "must not run"}),
    );
    assert!(
        (400..500).contains(&status),
        "create must fail closed: {status} {body}"
    );
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("runc") || error.contains("rootfs"),
        "error must name the missing sandbox runtime: {body}"
    );
    // Rejected admission leaves no execution index and no session row.
    let (status, body) = get(&fleet, &format!("/api/sessions/{PREFLIGHT_SESSION}"));
    assert_eq!(status, 404, "no session may exist: {body}");
    assert_eq!(stub.request_count(), 0, "no model call may happen");
}

/// The full sandbox contract under runc: every turn of a `run_mode: agent`
/// session runs the rootfs-installed session runner in a read-only OCI
/// container with the pinned pool bound at `/workspace/agent`; the host
/// tails the runner's Say frames into the store, so the operator-facing
/// surfaces look exactly like a host session. Skips without runc.
#[test]
fn sandbox_agent_session_runs_and_continues_in_runc() {
    if !runc_available() {
        eprintln!("SKIP: runc unavailable");
        return;
    }
    let reply1 = format!("{REPLY1}\n```json\n{{\"turn\": 1}}\n```\n");
    let reply2 = format!("{REPLY2}\n```json\n{{\"turn\": 2}}\n```\n");
    let stub = LlmStub::spawn(vec![
        Script::dynamic(move |_| reply1.clone()),
        Script::dynamic(move |_| reply2.clone()),
    ]);
    let tmp = tempfile::tempdir().unwrap();
    write_card(
        tmp.path(),
        "boxer",
        "boxer-pool",
        "agent",
        &format!("# boxer\n{SOUL_MARKER}\n"),
        Some(("boxer-skills", "probe")),
    );
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-sbx-runc");
    fleet.wait_ready(&["operator", "dag", "agent"]);
    prepare_rootfs(&fleet);

    // Turn 1: one call creates the session and runs the first container round.
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": RUNC_SESSION, "kind": "agent", "agent": "boxer",
                "node_id": fleet.node_id(), "prompt": PROMPT1}),
    );
    assert_eq!(status, 200, "create sandbox session: {body}");
    let doc = fleet.wait_idle(RUNC_SESSION);
    assert_eq!(doc["result"]["session_id"], RUNC_SESSION, "result: {doc}");
    assert!(
        out_text(&doc).contains(REPLY1),
        "turn-1 output_text: {}",
        out_text(&doc)
    );
    assert_eq!(doc["result"]["output_json"], json!({"turn": 1}));

    // The session projection is a first-class chat session naming the card.
    let (status, detail) = get(&fleet, &format!("/api/sessions/{RUNC_SESSION}"));
    assert_eq!(status, 200, "session detail: {detail}");
    assert_eq!(detail["meta"]["agent"], "boxer", "meta: {}", detail["meta"]);
    let transcript = detail["messages"].to_string();
    assert!(
        contains_all(&transcript, &[PROMPT1, REPLY1]),
        "transcript: {transcript}"
    );

    // Say frames: the host tailed the container's events.ndjson into the store.
    let frames1 = sse_read(
        &fleet.base,
        &format!("/api/sessions/{RUNC_SESSION}/events"),
        TOKEN,
        0,
        None,
    );
    let deltas1 = text_deltas(&frames1);
    assert!(deltas1.contains(REPLY1), "turn-1 deltas: {deltas1}");
    assert!(
        frames1.iter().any(|frame| frame.event == "done"),
        "no done frame"
    );
    assert_eq!(
        frames1.last().map(|frame| frame.event.as_str()),
        Some("stream_end")
    );

    // In-container pool resolution: soul.md composed the system prompt inside the container.
    let requests = stub.wait_for_requests(1);
    assert!(
        requests[0].contains(SOUL_MARKER),
        "container prompt: {}",
        requests[0]
    );
    assert!(
        requests[0].contains(PROMPT1),
        "turn prompt: {}",
        requests[0]
    );

    // Bundle shape: direct runner argv, session contract env, read-only pinned-pool bind.
    let bundles = fleet.node_data.join("dag/bundles/agent-sessions");
    let bundle = read_json(&bundles.join(RUNC_SESSION).join("config.json"));
    assert_eq!(
        bundle["process"]["args"],
        json!(["/usr/bin/agent-session-runner"]),
        "bundle process.args: {bundle}"
    );
    let env = bundle["process"]["env"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        env.contains(&json!("OPENCODER_AGENTS_DIR=/workspace/agent")),
        "agents dir env: {env:?}"
    );
    assert!(
        env.contains(&json!(format!("OPENCODER_STEP_SESSION_ID={RUNC_SESSION}"))),
        "session id env: {env:?}"
    );
    assert!(
        env.iter()
            .any(|e| e.as_str().unwrap_or("").starts_with("OPENAI_BASE_URL=")),
        "llm endpoint env: {env:?}"
    );
    let agents_mount = bundle["mounts"]
        .as_array()
        .and_then(|mounts| {
            mounts
                .iter()
                .find(|m| m["destination"] == json!("/workspace/agent"))
        })
        .unwrap_or_else(|| panic!("no /workspace/agent mount in bundle"));
    assert!(
        agents_mount["options"]
            .as_array()
            .is_some_and(|options| options.contains(&json!("ro"))),
        "agents mount must be read-only: {agents_mount}"
    );

    // Runner artifacts on the host through the rw session dir.
    let session_dir = fleet
        .node_data
        .join("dag")
        .join(RUNC_SESSION)
        .join("session");
    let session = read_json(&session_dir.join("session.json"));
    assert_eq!(
        session["session_id"], RUNC_SESSION,
        "session.json: {session}"
    );
    assert_eq!(session["status"], json!("done"), "session.json: {session}");
    assert_eq!(
        read_json(&session_dir.join("output.json")),
        json!({"turn": 1}),
        "container output.json"
    );

    // Turn 2: the follow-up prompt re-launches the workload — a NEW round continues the session.
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/sessions/{RUNC_SESSION}/prompt"),
        &json!({"prompt": PROMPT2, "input_id": "e2e-turn2"}),
    );
    assert_eq!(status, 200, "sandbox follow-up prompt: {body}");
    let doc = fleet.wait_idle(RUNC_SESSION);
    assert!(
        out_text(&doc).contains(REPLY2),
        "turn-2 output_text: {}",
        out_text(&doc)
    );
    assert_eq!(doc["result"]["output_json"], json!({"turn": 2}));

    // Round 2 was seeded with the durable transcript of round 1.
    let requests = stub.wait_for_requests(2);
    assert!(
        contains_all(&requests[1], &[REPLY1, PROMPT2]),
        "continuation request: {}",
        requests[1]
    );
    let (status, detail) = get(&fleet, &format!("/api/sessions/{RUNC_SESSION}"));
    assert_eq!(status, 200, "session detail: {detail}");
    let transcript = detail["messages"].to_string();
    assert!(
        contains_all(&transcript, &[PROMPT1, REPLY1, PROMPT2, REPLY2]),
        "transcript accumulates both turns: {transcript}"
    );

    // The event surface grew across the two turns and carries turn 2.
    let frames2 = sse_read(
        &fleet.base,
        &format!("/api/sessions/{RUNC_SESSION}/events"),
        TOKEN,
        0,
        None,
    );
    assert!(
        frames2.len() > frames1.len(),
        "events grow: {} then {}",
        frames1.len(),
        frames2.len()
    );
    assert!(
        text_deltas(&frames2).contains(REPLY2),
        "turn-2 deltas: {}",
        text_deltas(&frames2)
    );
}

/// Regression guard (no runc needed): `run_mode: "operator"` cards keep the O5 host loop.
#[test]
fn operator_mode_card_stays_on_host() {
    let stub = LlmStub::spawn_text(&[HOST_REPLY]);
    let tmp = tempfile::tempdir().unwrap();
    write_card(
        tmp.path(),
        "hostly",
        "hostly-pool",
        "operator",
        &format!("# hostly\n{HOST_SOUL_MARKER}\n"),
        None,
    );
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-sbx-host");
    fleet.wait_ready(&["operator", "dag", "agent"]);
    // No rootfs provisioned — must not matter: operator-mode keeps the host loop.

    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": HOST_SESSION, "kind": "agent", "agent": "hostly",
                "node_id": fleet.node_id(), "prompt": HOST_PROMPT}),
    );
    assert_eq!(status, 200, "create host card session: {body}");
    let doc = fleet.wait_idle(HOST_SESSION);
    assert!(
        out_text(&doc).contains(HOST_REPLY),
        "output_text: {}",
        out_text(&doc)
    );
    // The pinned pool resolved on the host too: its soul marker composed the turn.
    let requests = stub.wait_for_requests(1);
    assert!(
        contains_all(&requests[0], &[HOST_SOUL_MARKER, HOST_PROMPT]),
        "host turn request: {}",
        requests[0]
    );
}
