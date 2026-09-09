//! Minimal closed-loop coverage for the two dispatch chains against a REAL
//! control app + REAL worker nodes (WS fleet): a saved DAG definition
//! dispatched through `/api/dag/defs/:id/dispatch` and driven to `done`
//! with run view / events / artifacts, and a team definition executed to a
//! final summary with member sessions and on-disk team state.

use super::*;

fn completed(text: String) -> Vec<LlmEvent> {
    // The DAG agent step captures its transcript from TextDelta frames, so
    // scripted answers need the delta plus the terminal Completed event.
    vec![
        LlmEvent::TextDelta(text.clone()),
        LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        },
    ]
}

/// Buffer one SSE response to text; the stream ends once the node reports
/// the execution finished, so a bounded wait is enough.
async fn sse_text(fleet: &Fleet, path: &str) -> String {
    let response = fleet.response("GET", path).await;
    assert_eq!(response.status(), 200);
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        axum::body::to_bytes(response.into_body(), opencoder_core::fleet::MAX_FRAME_BYTES),
    )
    .await
    .expect("sse stream must finish")
    .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn dag_saved_definition_dispatch_runs_to_done() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    // The agent step answers with prose around a ```json fence so the run
    // loop recovers structured output via `extract_output_json_from`.
    client.queue_script(completed(
        "here is the result\n```json\n{\"answer\":\"saved-def-ok\"}\n```\ndone"
            .to_string(),
    ));
    let saved = fleet
        .call(
            "POST",
            "/api/dag/defs",
            json!({"spec":{"name":"saved-loop","steps":[
                {"name":"answer","kind":{"type":"agent","prompt":"produce the structured answer"}}
            ]}}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let dispatched = fleet
        .call("POST", "/api/dag/defs/saved-loop/dispatch", json!({"id":"dag-saved-loop"}))
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");
    assert_eq!(dispatched.body["run_id"], json!("dag-saved-loop"));
    assert_eq!(dispatched.body["execution"]["id"], json!("dag-saved-loop"));

    let detail = settled(&fleet.nodes[0], "dag-saved-loop").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");

    // Run view: control routes the inspect through the node and projects the
    // definition snapshot taken at dispatch time.
    let run = fleet
        .call("GET", "/api/dag/runs/dag-saved-loop", Value::Null)
        .await;
    assert_eq!(run.status, 200, "{run:?}");
    assert_eq!(run.body["status"], json!("done"));
    assert_eq!(run.body["dag_id"], json!("saved-loop"));
    let steps = run.body["spec"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["name"], json!("answer"));

    // Events: real execution frames persisted by the worker, replayed as SSE.
    let text = sse_text(&fleet, "/api/dag/runs/dag-saved-loop/events?after=0").await;
    for kind in ["run_started", "step_done", "run_finished"] {
        assert!(
            text.contains(&format!("event: {kind}")),
            "missing {kind} in {text}"
        );
    }

    // Artifact contract: `<workflow_root>/<run>/<step>/output.json` holds the
    // fenced JSON recovered from the agent transcript.
    let output_path = fleet
        .root()
        .join("n0/node/dag/dag-saved-loop/answer/output.json");
    let output: Value =
        serde_json::from_str(&std::fs::read_to_string(&output_path).unwrap()).unwrap();
    assert_eq!(output, json!({"answer":"saved-def-ok"}));
    fleet.shutdown().await;
}

#[tokio::test]
async fn team_dispatch_completes_with_final_summary() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let saved = fleet
        .call(
            "POST",
            "/api/teams",
            json!({"name":"loop-team","captain":"captain","members":[
                {"id":"captain","agent":"act","role":"coordinate"},
                {"id":"reviewer","agent":"act","role":"review"}
            ]}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    // plan decision → member answer → summary decision → closing decision.
    for text in [
        json!({"question":"review the delivery","participants":["reviewer"],"rationale":"need a review"})
            .to_string(),
        "member reviewed the delivery".into(),
        "{\"summary\":\"review finished\",\"aligned\":true}".into(),
        "{\"complete\":true,\"final_summary\":\"team loop complete\"}".into(),
    ] {
        client.queue_script(completed(text));
    }
    let dispatched = fleet
        .call(
            "POST",
            "/api/executions",
            json!({"id":"team-loop-1","kind":"team","target":"loop-team","input":{"prompt":"review the delivery"}}),
        )
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");
    let detail = settled(&fleet.nodes[0], "team-loop-1").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");

    // The member turn ran as its own indexed session on the executing node.
    assert!(fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .iter()
        .any(|index| index.id.starts_with("member-")));

    // Worker-side team state: team.json under the team dir and the topic
    // record carrying the final summary from the closing decision.
    let team_state = fleet.root().join("n0/node/team/team-loop-1/team/loop-team");
    let team: Value = serde_json::from_str(
        &std::fs::read_to_string(team_state.join("team.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(team["name"], json!("loop-team"));
    let topic: Value = serde_json::from_str(
        &std::fs::read_to_string(team_state.join("team-loop-1/team.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(topic["final_summary"], json!("team loop complete"));
    fleet.shutdown().await;
}
