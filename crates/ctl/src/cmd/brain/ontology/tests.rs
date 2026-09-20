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
    std::fs::write(
        &context,
        json!({
            "schema_version":3,"run_id":"brain-configured-planner","generation":1,"round":0,
            "request":{"schema_version":3,"objective":"test finite decision"},
            "capabilities":[],"operations":[],"summaries":{}
        })
        .to_string(),
    )
    .unwrap();
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
