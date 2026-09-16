//! M1 — the full team face: register a team through `POST /api/teams`, run a
//! topic through `POST /api/executions`, and verify the finished topic, the
//! turn ledger, the on-disk team artifacts and the member session indexes.

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script};

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("json file")).expect("json payload")
}
use serde_json::{json, Value};

const RUN: &str = "team-e2e-flow-1";
const TEAM: &str = "review";
fn definition() -> Value {
    json!({
        "name": TEAM,
        "captain": "act",
        "members": [
            {"agent": "act"},
            {"agent": "plan", "capabilities": ["review implementation"]}
        ]
    })
}

/// The team runtime's three captain decisions share one shape in this stub:
/// plan {question, participants, rationale}, summary {summary, aligned},
/// closing {complete, final_summary}. A single aligned+complete verdict
/// finishes the topic in one turn.
fn decision_script() -> Script {
    Script::dynamic(|_body| {
        json!({
            "question": "inspect",
            "participants": ["plan"],
            "rationale": "one reviewer is enough",
            "summary": "aligned",
            "aligned": true,
            "complete": true,
            "final_summary": "team completed"
        })
        .to_string()
    })
}

#[test]
fn team_topic_runs_to_completion_with_turn_ledger() {
    // Every call — plan, member answer, summary, closing — gets the same
    // aligned+complete verdict, so one turn finishes the topic.
    let stub = LlmStub::spawn(vec![decision_script(); 4]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "team-flow-node");
    fleet.wait_ready(&["team"]);

    // Member capabilities are control-plane state: register a brain
    // capability bound to the "plan" agent so the dispatch-time freeze
    // replaces the definition's capability list with the bound summary.
    const SUMMARY: &str = "review-implementation-plan";
    let (status, body) = fleet.http(
        "POST",
        "/api/brain/capabilities",
        &json!({
            "capability_type": "tool-usage",
            "summary": SUMMARY,
            "input_desc": "a change to review",
            "output_desc": "a review verdict",
            "eng_inputs": []
        }),
    );
    assert_eq!(status, 201, "create capability: {body}");
    let capability_id = body["capability"]["id"].as_str().expect("capability id");
    let (status, body) = fleet.http(
        "PUT",
        &format!("/api/brain/capabilities/{capability_id}/target"),
        &json!({"kind": "agent", "target": "plan"}),
    );
    assert_eq!(status, 200, "bind capability: {body}");

    // Register the team definition on the fleet catalog.
    let (status, body) = fleet.http("POST", "/api/teams", &definition());
    assert_eq!(status, 200, "save team: {body}");
    assert_eq!(body["name"], json!(TEAM));
    assert_eq!(body["captain"], json!("act"));
    let (status, body) = fleet.http("GET", "/api/teams", &json!({}));
    assert_eq!(status, 200);
    assert!(
        body["teams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|team| team["name"] == json!(TEAM)),
        "team catalog must list the definition: {body}"
    );

    // Run a topic: the definition resolves from the fleet catalog via target.
    let (status, body) = fleet.http(
        "POST",
        "/api/executions",
        &json!({
            "id": RUN,
            "kind": "team",
            "target": TEAM,
            "input": {"prompt": "review change"}
        }),
    );
    assert_eq!(status, 202, "dispatch team: {body}");
    assert_eq!(body["kind"], json!("team"));
    assert_eq!(body["node_id"], json!(fleet.node_id()));

    // Four model calls: captain plan, member answer, captain summary,
    // captain closing. The capability-carrying member prompt must reach the
    // member session.
    let requests = stub.wait_for_requests(4);
    assert!(
        requests.iter().any(|body| body.contains("review change")),
        "requirement must reach the team prompts: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|body| body.contains(&format!("你的能力：{SUMMARY}"))),
        "member prompt must carry the capability prefix: {requests:?}"
    );

    let (_, dbg) = fleet.http("GET", &format!("/api/executions/{RUN}"), &json!({}));
    eprintln!("DEFINITION: {}", dbg["definition"]);
    let doc = fleet.wait_terminal(RUN);
    assert_eq!(doc["execution"]["status"], json!("done"), "inspect: {doc}");
    assert_eq!(doc["topic"]["final_summary"], json!("team completed"));
    assert_eq!(doc["topic"]["finish_reason"], json!("complete"));
    assert_eq!(doc["topic"]["status"], json!("finished"));

    // Turn ledger: one aligned turn with the planned participant.
    let (status, body) = fleet.http(
        "GET",
        &format!("/api/executions/{RUN}/team-turns?after_turn=0"),
        &json!({}),
    );
    assert_eq!(status, 200, "team turns: {body}");
    assert_eq!(body["more"], json!(false));
    // Turns are 1-based; the page cursor (after_turn) is exclusive.
    let turns = body["turns"].as_array().expect("turns array");
    assert_eq!(turns.len(), 1, "turns: {body}");
    assert_eq!(turns[0]["turn"], json!(1));
    assert_eq!(turns[0]["meta"]["aligned"], json!(true));
    assert_eq!(turns[0]["meta"]["participants"], json!(["plan"]));
    assert_eq!(turns[0]["meta"]["question"], json!("inspect"));

    // On-disk artifacts: team record + topic metadata under the node layout.
    let team_root = fleet.node_data.join("team").join(RUN).join("team");
    let team_record = read_json(&team_root.join(TEAM).join("team.json"));
    assert_eq!(team_record["name"], json!(TEAM));
    assert_eq!(team_record["captain"]["node_id"], json!("act"));
    let topic = read_json(&team_root.join(TEAM).join(RUN).join("team.json"));
    assert_eq!(topic["title"], json!(TEAM));
    assert_eq!(topic["requirement"], json!("review change"));
    assert_eq!(topic["status"], json!("finished"));
    assert_eq!(topic["final_summary"], json!("team completed"));

    // Member (and captain) sessions are indexed as agent executions.
    let (status, body) =
        fleet.http("GET", "/api/executions?kind=agent&limit=200", &json!({}));
    assert_eq!(status, 200, "agent indexes: {body}");
    let members = body["executions"]
        .as_array()
        .expect("executions array")
        .iter()
        .filter(|row| row["id"].as_str().unwrap_or_default().starts_with("member-"))
        .count();
    assert!(members >= 4, "expected the 4 team sessions, got {members}");

    let (status, detail) = fleet.http("GET", &format!("/api/executions/{RUN}"), &json!({}));
    assert_eq!(status, 200, "inspect: {detail}");
    assert_eq!(detail["result"]["final_summary"], json!("team completed"));

    // The pinned definition froze the bound capability summary.
    assert_eq!(
        doc["definition"]["members"][1]["capabilities"],
        json!([SUMMARY]),
        "definition: {doc}"
    );

    // The topic metadata is also the execution's result payload.
    let (status, detail) = fleet.http("GET", &format!("/api/executions/{RUN}"), &json!({}));
    assert_eq!(status, 200, "inspect: {detail}");
    assert_eq!(
        detail["result"]["final_summary"],
        json!("team completed")
    );
}
