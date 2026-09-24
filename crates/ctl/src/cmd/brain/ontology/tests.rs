use super::*;
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};

#[tokio::test]
async fn isolated_activation_sends_configured_reasoning_to_the_provider() {
    let (sent, mut received) = tokio::sync::mpsc::channel::<Value>(1);
    let app = Router::new().route("/chat/completions", post(
        |State(sent): State<tokio::sync::mpsc::Sender<Value>>, Json(body): Json<Value>| async move {
            sent.send(body).await.unwrap();
            let decision = json!({"decision":"fail","reason":"test bounded decision","error_type":"test"}).to_string();
            let event = json!({"choices":[{"index":0,"delta":{"content":decision},"finish_reason":null}]}).to_string();
            let end = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}).to_string();
            ([("content-type", "text/event-stream")], format!("data: {event}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
        },
    )).with_state(sent);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let context = directory.path().join("context.json");
    let config = directory.path().join("config.json");
    let output = directory.path().join("decision.json");
    std::fs::write(&context, layered_context(node_context()).to_string()).unwrap();
    std::fs::write(&config, json!({
        "model":"fixture/planner","providers":{"fixture":{"base_url":format!("http://{address}"),"api_key":"fixture"}},
        "reasoning_effort":"low"
    }).to_string()).unwrap();
    assert_eq!(activate(&context, &config, &output).await.unwrap(), 0);
    let request = received.recv().await.unwrap();
    assert_eq!(request["reasoning_effort"], "low");
    assert_eq!(request["model"], "planner");
    assert_eq!(request["max_tokens"], 16384);
    let decision: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(decision["decision"], "fail");
    server.abort();
    let _ = server.await;
}

const LAYERED_CONTRACT_MARKER: &str = "Complete only after the final layer passes";

/// Layer context for the CLI activation tests: one node, one layer.
fn layered_context(nodes: Value) -> Value {
    json!({
        "schema_version": 7,
        "run_id": "brain-layered-cli",
        "generation": 7,
        "layer": 1,
        "total_layers": 1,
        "request": {
            "schema_version": 7,
            "plan": {
                "schema_version": 7,
                "title": "layered plan",
                "objective": "ship the layered canvas",
                "inputs": {},
                "layers": [{"layer_id":"first","title":"first","objective":"perform","success_criteria":"verified"}],
                "nodes": [{"node_id": "n1", "layer_id":"first", "title": "first", "capability_id": "cap-1", "objective":"perform"}],
                "transitions": []
            },
            "inputs": {}
        },
        "capabilities": nodes.as_array().unwrap().iter().map(|node| node["capability"].clone()).collect::<Vec<_>>(),
        "summaries": {},
        "operations": []
    })
}

fn node_context() -> Value {
    json!([{
        "node_id": "n1",
        "title": "first",

        "retry_max_attempts": 2,
        "capability": {
            "capability_id": "cap-1",
            "kind": "agent",
            "target": "solver",
            "input_desc": "task",
            "output_desc": "result",
            "required_inputs": [],
            "definition": {},
            "version": "1"
        },
        "upstream": [],
        "downstream": []
    }])
}

/// Streaming stub that answers `decision` once and reports the provider body.
async fn layered_provider(
    decision: Value,
) -> (
    String,
    tokio::sync::mpsc::Receiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let (sent, received) = tokio::sync::mpsc::channel::<Value>(1);
    let app = Router::new().route(
        "/chat/completions",
        post(
            |State((sent, decision)): State<(tokio::sync::mpsc::Sender<Value>, Value)>,
             Json(body): Json<Value>| async move {
                sent.send(body).await.unwrap();
                let text = decision.to_string();
                let event = json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}).to_string();
                let end = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}).to_string();
                ([("content-type", "text/event-stream")], format!("data: {event}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
            },
        ),
    )
    .with_state((sent, decision));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), received, server)
}

struct Fixture {
    context: std::path::PathBuf,
    config: std::path::PathBuf,
    output: std::path::PathBuf,
    request: tokio::sync::mpsc::Receiver<Value>,
    server: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}

/// Context + config + output files wired to a one-shot stub provider.
async fn fixture(context: Value, decision: Value) -> Fixture {
    let (base_url, request, server) = layered_provider(decision).await;
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture {
        context: directory.path().join("context.json"),
        config: directory.path().join("config.json"),
        output: directory.path().join("decision.json"),
        request,
        server,
        _directory: directory,
    };
    std::fs::write(&fixture.context, context.to_string()).unwrap();
    std::fs::write(
        &fixture.config,
        json!({
            "model": "fixture/planner",
            "providers": {"fixture": {"base_url": base_url, "api_key": "fixture"}},
            "reasoning_effort": "low"
        })
        .to_string(),
    )
    .unwrap();
    fixture
}

#[tokio::test]
async fn layered_context_sends_the_layer_contract_and_writes_the_decision() {
    let decision = json!({
        "decision": "dispatch_layer",
        "layer": 1,
        "assignments": [{"node_id": "n1", "inputs": {}, "reason": "start"}],
        "reason": "one ready node",
        "evidence_execution_ids": []
    });
    let mut fixture = fixture(layered_context(node_context()), decision).await;
    assert_eq!(
        activate(&fixture.context, &fixture.config, &fixture.output)
            .await
            .unwrap(),
        0
    );
    let request = fixture.request.recv().await.unwrap();
    assert_eq!(request["model"], "planner");
    assert_eq!(request["reasoning_effort"], "low");
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains(LAYERED_CONTRACT_MARKER));
    let instruction: Value =
        serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(instruction["schema_version"], 7);
    assert_eq!(instruction["plan"]["title"], "layered plan");
    assert_eq!(instruction["plan"]["nodes"][0]["node_id"], "n1");
    assert_eq!(instruction["capabilities"][0]["capability_id"], "cap-1");
    let written: Value = serde_json::from_slice(&std::fs::read(&fixture.output).unwrap()).unwrap();
    assert_eq!(written["decision"], "dispatch_layer");
    assert_eq!(written["assignments"][0]["node_id"], "n1");
    fixture.server.abort();
}

#[tokio::test]
async fn final_context_retains_the_same_reflection_contract() {
    let decision =
        json!({"decision": "complete", "reason": "all nodes done", "summary": "canvas shipped"});
    let mut fixture = fixture(layered_context(node_context()), decision).await;
    assert_eq!(
        activate(&fixture.context, &fixture.config, &fixture.output)
            .await
            .unwrap(),
        0
    );
    let request = fixture.request.recv().await.unwrap();
    let messages = request["messages"].as_array().unwrap();
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains(LAYERED_CONTRACT_MARKER));
    let instruction: Value =
        serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(instruction["schema_version"], 7);
    let written: Value = serde_json::from_slice(&std::fs::read(&fixture.output).unwrap()).unwrap();
    assert_eq!(written["decision"], "complete");
    assert_eq!(written["summary"], "canvas shipped");
    fixture.server.abort();
}

fn run_plan(body: &str) -> RequestPlan {
    runs(&RunsCmd::Create { json: body.into() }).unwrap()
}

/// Schema 3 stays byte-identical, schema 4 is accepted, everything else is an
/// explicit plan-time error (never a silent fallback to v3).
#[test]
fn run_create_accepts_only_layered_schema() {
    let v4 = run_plan(r#"{"schema_version":7,"plan":{"schema_version":7}}"#);
    assert_eq!(v4.method, reqwest::Method::POST);
    assert_eq!(v4.path, "/api/brain/runs");
    assert_eq!(v4.body.unwrap()["schema_version"], 7);

    for raw in [
        r#"{"schema_version":3}"#,
        r#"{"schema_version":2,"plan_id":"p1"}"#,
        r#"{"schema_version":4}"#,
        r#"{"plan_id":"p1"}"#,
        r#"{"schema_version":"4"}"#,
    ] {
        let error = runs(&RunsCmd::Create { json: raw.into() }).unwrap_err();
        assert!(
            error.to_string().contains("schema_version: 7"),
            "unexpected error for {raw}: {error}"
        );
    }
}

/// The layered routes are the v4 read surface; the v3 round route is untouched.
#[test]
fn run_reads_map_to_the_locked_paths() {
    let plan = runs(&RunsCmd::Layered { id: "r1".into() }).unwrap();
    assert_eq!(plan.method, reqwest::Method::GET);
    assert_eq!(plan.path, "/api/brain/runs/r1/layered");
    assert_eq!(plan.body, None);

    let plan = runs(&RunsCmd::LayeredRound {
        id: "r1".into(),
        round: 2,
    })
    .unwrap();
    assert_eq!(plan.method, reqwest::Method::GET);
    assert_eq!(plan.path, "/api/brain/runs/r1/layered/rounds/2");
}
