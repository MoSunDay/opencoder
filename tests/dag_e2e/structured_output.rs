//! D1b — agent-step structured-output extraction, end to end. The LLM stub
//! replays the two output-contract forms for two chained agent steps: the
//! v3 primary ```json fence and the bare-JSON-after-narration fallback the
//! whole-text parse could never recover. Both must land a non-null
//! `output.json` with reachable business fields.

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script, EXTRA_REPLY};
use serde_json::{json, Value};

const DEF: &str = "e2e-structured";
const RUN: &str = "dag-e2e-structured-1";

const FENCED_REPLY: &str = "分析完成。\n```json\n{\"depend_type\": \"strong\", \
     \"analysis_report\": {\"summary\": \"fenced\"}}\n```\n";
const BARE_REPLY: &str = "## 分析过程\n调用链定位到 handler，签名匹配……（长叙述，无围栏）\n最终结论：\n{\"depend_type\": \"weak\", \
     \"analysis_report\": {\"summary\": \"bare\"}}\n";

fn spec() -> Value {
    json!({
        "name": DEF,
        "description": "structured output contract forms",
        "steps": [
            {"name": "fenced", "kind": {"type": "agent", "prompt": "给出围栏结论"}},
            {"name": "bare", "depends_on": ["fenced"],
             "kind": {"type": "agent", "prompt": "给出裸 JSON 结论"}},
        ],
    })
}

fn read_json_at(path: &std::path::Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// Node artifact `<node-data>/dag/<run>/<step>/output.json`.
fn step_json(fleet: &Fleet, run: &str, step: &str) -> Value {
    read_json_at(
        &fleet
            .node_data
            .join("dag")
            .join(run)
            .join(step)
            .join("output.json"),
    )
}

/// Content-keyed replies: a step sub-session also fires a best-effort title
/// pass that replays the same user message, so order-based FIFO scripts
/// drift; the prompt text is the discriminator instead.
fn reply_by_prompt() -> Script {
    Script::dynamic(|body| {
        let last = body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default();
        if last.contains("给出裸 JSON 结论") {
            BARE_REPLY.into()
        } else if last.contains("给出围栏结论") {
            FENCED_REPLY.into()
        } else {
            EXTRA_REPLY.into()
        }
    })
}

#[test]
fn agent_step_extracts_fenced_and_bare_json_output() {
    // Two steps x (main turn + title pass), plus slack; content-keyed so
    // slot order never matters.
    let stub = LlmStub::spawn(vec![reply_by_prompt(); 8]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({}),
        "dag-structured-node",
    );

    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec()}));
    assert_eq!(status, 200, "save def: {body}");
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{DEF}/dispatch"),
        &json!({"id": RUN}),
    );
    assert_eq!(status, 202, "dispatch {RUN}: {body}");

    let doc = fleet.wait_terminal(RUN);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");

    let (status, progress) =
        fleet.http("GET", &format!("/api/dag/runs/{RUN}/progress"), &json!({}));
    assert_eq!(status, 200);
    assert_eq!(progress["total"], 2);
    assert_eq!(progress["done"], 2);
    assert_eq!(progress["error"], 0, "progress: {progress}");

    // Primary form: the fence is extracted; output.json is NOT null.
    let fenced = step_json(&fleet, RUN, "fenced");
    assert!(fenced.is_object(), "fenced output.json: {fenced}");
    assert_eq!(fenced["depend_type"], json!("strong"));
    assert_eq!(fenced["analysis_report"]["summary"], json!("fenced"));

    // Fallback form: narration + bare JSON tail (no fence anywhere) must
    // still produce a non-null output.json — the regression this suite pins.
    let bare = step_json(&fleet, RUN, "bare");
    assert!(bare.is_object(), "bare output.json must not be null: {bare}");
    assert_eq!(bare["depend_type"], json!("weak"));
    assert_eq!(bare["analysis_report"]["summary"], json!("bare"));

    // Both steps closed done with the extraction serving the artifact IO.
    for step in ["fenced", "bare"] {
        let meta = read_json_at(&fleet.node_data.join("dag").join(RUN).join(step).join("meta.json"));
        assert_eq!(meta["outcome"], "done", "{step} meta: {meta}");
    }

    let requests = stub.wait_for_requests(2);
    let fenced_reqs: Vec<&String> = requests.iter().filter(|r| r.contains("给出围栏结论")).collect();
    let bare_reqs: Vec<&String> = requests.iter().filter(|r| r.contains("给出裸 JSON 结论")).collect();
    assert!(!fenced_reqs.is_empty(), "fenced prompt never sent: {requests:?}");
    assert!(!bare_reqs.is_empty(), "bare prompt never sent: {requests:?}");
}
