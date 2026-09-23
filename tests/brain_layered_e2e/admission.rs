//! Process-level admission gates: the historical brain writers stay closed,
//! the layered protocol is never a fallback for an unknown schema version, and
//! an invalid canvas is rejected before any node or model work.
use crate::fixtures::{plan, request, RUN};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;
use serde_json::{json, Value};

fn fleet(tmp: &tempfile::TempDir, stub: &LlmStub, name: &str) -> Fleet {
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), name);
    fleet.wait_ready(&["brain"]);
    fleet
}

#[test]
fn raw_brain_submissions_stay_rejected_for_the_layered_canvas() {
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = fleet(&tmp, &stub, "layered-guard-node");

    let (status, body) = fleet.http(
        "POST",
        "/api/executions",
        &json!({"id":"brain-e2e-bypass","kind":"brain","input":{"schema_version":6}}),
    );
    assert_eq!(status, 409, "bypass guard: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("schema_version: 6"),
        "the layered canvas keeps the run endpoint: {body}"
    );
    assert_eq!(stub.request_count(), 0, "no model traffic");
}

#[test]
fn unknown_schema_versions_never_fall_back_to_a_writer() {
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = fleet(&tmp, &stub, "layered-schema-node");

    // Only schema 6 owns a writer: an
    // absent or unknown version is an explicit error.
    for version in [None, Some(0), Some(2), Some(3), Some(5)] {
        let mut body = request(RUN);
        match version {
            Some(version) => body["schema_version"] = json!(version),
            None => {
                body.as_object_mut().unwrap().remove("schema_version");
            }
        }
        let (status, reply) = fleet.http("POST", "/api/brain/runs", &body);
        assert_eq!(status, 409, "version {version:?}: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or_default()
                .contains("schema_version: 6"),
            "version {version:?} must name the migration: {reply}"
        );
        // A rejected submission leaves no projection behind.
        let (status, missing) =
            fleet.http("GET", &format!("/api/brain/runs/{RUN}/layered"), &json!({}));
        assert_eq!(status, 404, "version {version:?}: {missing}");
        assert_eq!(missing["error"], json!("layered run not found"));
    }
    assert_eq!(stub.request_count(), 0, "rejection must not dispatch");
}

#[test]
fn invalid_canvases_are_rejected_before_any_dispatch() {
    let stub = LlmStub::spawn(vec![]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = fleet(&tmp, &stub, "layered-admission-node");

    let mut unknown_capability = request(RUN);
    unknown_capability["plan"]["nodes"][0]["capability_ids"][0] =
        json!("not-registered-capability");

    let mut cyclic = request(RUN);
    cyclic["plan"]["edges"] = json!([{"from":"scan","to":"apply"},{"from":"apply","to":"scan"}]);

    let mut too_deep = request(RUN);
    too_deep["depth"] = json!(4);

    let mut orphan = request(RUN);
    orphan["depth"] = json!(1);

    let cases: [(&str, Value, &str); 5] = [
        (
            "id",
            request("layered-missing-prefix"),
            "invalid brain run id",
        ),
        (
            "capability",
            unknown_capability,
            "node capability unavailable: not-registered-capability",
        ),
        (
            "configured return",
            cyclic,
            "remove configured return edges",
        ),
        ("depth", too_deep, "nesting depth exceeded"),
        ("parent", orphan, "nested depth and parent must agree"),
    ];
    for (label, body, expected) in cases {
        let (status, reply) = fleet.http("POST", "/api/brain/runs", &body);
        assert_eq!(status, 400, "{label}: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap_or_default()
                .contains(expected),
            "{label} must be refused with `{expected}`: {reply}"
        );
    }
    // Nothing was admitted, so nothing can be read back or dispatched.
    let (status, missing) =
        fleet.http("GET", &format!("/api/brain/runs/{RUN}/layered"), &json!({}));
    assert_eq!(status, 404, "rejected canvases leave no run: {missing}");
    assert_eq!(missing["error"], json!("layered run not found"));
    assert_eq!(stub.request_count(), 0, "admission gates never wake a node");
    // The plan fixture is the same document the accepted scenarios submit, so
    // the gates above are the only reason these submissions failed.
    assert_eq!(plan()["nodes"].as_array().unwrap().len(), 2);
}
