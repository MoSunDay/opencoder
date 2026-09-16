//! O5 — the agent-kind session face: `POST /api/sessions` with
//! `kind=agent` creates the agent execution AND its session in one call
//! (default `agent-` id prefix, no operator preamble), the declared
//! `how_append` rides the harness env like a DAG agent step and lands in
//! the pinned prompt pool's `how.md`, and the idle fold exposes the
//! bounded output contract (`session_id` + `output_text` +
//! `output_json` from the ```json fence in the transcript tail).

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::json;
use std::path::{Path, PathBuf};

const PROMPT: &str = "e2e-agent-prompt: 汇总调查结论";
const REPLY: &str = "e2e-agent-reply";
const HOW_APPEND: &str = "e2e how note: always answer with a json fence";
const SESSION: &str = "agent-e2e-flow-1";

/// Every `prompts/<pool>/v*/how.md` under `root` (the pinned agents dir
/// layout is node-internal, so the assertion walks the whole workdir).
fn find_how_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_how_files(&path, found);
        } else if path.file_name().is_some_and(|name| name == "how.md") {
            found.push(path);
        }
    }
}

#[test]
fn agent_session_runs_prompt_and_exposes_output() {
    let reply = format!("{REPLY}\n```json\n{{\"answer\": 42}}\n```");
    let stub = LlmStub::spawn_text(&[reply.as_str()]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-agent-node");
    // The node serves the agent family alongside operator/dag.
    fleet.wait_ready(&["operator", "dag", "agent"]);

    // One call creates the session and its agent-kind execution.
    let (status, body) = fleet.http(
        "POST",
        "/api/sessions",
        &json!({"id": SESSION, "kind": "agent", "agent": "act", "node_id": fleet.node_id(),
                "prompt": PROMPT, "how_append": HOW_APPEND}),
    );
    assert_eq!(status, 200, "create agent session: {body}");
    assert_eq!(body["id"], SESSION);
    assert_eq!(body["execution"]["kind"], "agent");
    assert_eq!(body["execution"]["node_id"], fleet.node_id());

    // The agent session's success state is `idle` — same fold as operator.
    let doc = fleet.wait_idle(SESSION);
    assert_eq!(doc["execution"]["status"], "idle", "inspect: {doc}");
    assert_eq!(doc["execution"]["kind"], "agent");
    // The agent result contract: session pointer + bounded output tail +
    // the ```json fence extracted from that tail (same as DAG agent steps).
    assert_eq!(doc["result"]["session_id"], SESSION);
    let output_text = doc["result"]["output_text"].as_str().unwrap_or_default();
    assert!(output_text.contains(REPLY), "result: {}", doc["result"]);
    assert_eq!(doc["result"]["output_json"], json!({"answer": 42}));

    // The transcript carries the raw prompt and the reply — no operator
    // preamble — and the session meta names the executing agent.
    let (status, detail) = fleet.http("GET", &format!("/api/sessions/{SESSION}"), &json!({}));
    assert_eq!(status, 200, "session detail: {detail}");
    assert_eq!(detail["meta"]["agent"], "act");
    let transcript = detail["messages"].to_string();
    assert!(transcript.contains(PROMPT), "transcript: {transcript}");
    assert!(transcript.contains(REPLY), "transcript: {transcript}");
    assert!(
        !transcript.contains("Operator agent"),
        "no preamble: {transcript}"
    );

    // Exactly one model call: the agent prompt, unprefixed.
    let requests = stub.wait_for_requests(1);
    assert!(
        requests[0].contains(PROMPT),
        "prompt hit the LLM: {}",
        requests[0]
    );
    assert!(
        !requests[0].contains("Operator agent"),
        "no operator preamble on the wire: {}",
        requests[0]
    );

    // The chat-page listing merges the agent session next to operator ones.
    let (status, sessions) = fleet.http("GET", "/api/sessions", &json!({}));
    assert_eq!(status, 200, "sessions list: {sessions}");
    let rows = sessions["sessions"].as_array().expect("sessions");
    let row = rows
        .iter()
        .find(|row| row["id"] == SESSION)
        .expect("agent session listed in /api/sessions");
    assert_eq!(row["agent"], "act");

    // The declared how_append reached the session's tool env (injected at
    // creation like the DAG agent step) and was persisted to the pinned
    // prompt pool's how.md on success — warn-only, outcome unchanged.
    let mut how_files = Vec::new();
    find_how_files(&fleet.workdir, &mut how_files);
    assert!(
        how_files.iter().any(|path| std::fs::read_to_string(path)
            .unwrap_or_default()
            .contains(HOW_APPEND)),
        "how.md must carry the append: {how_files:?}"
    );
}
