//! CLI resolution and native request wire format, including history and auth failures.
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn todo_input_and_reads_use_the_same_native_execution() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let router=Router::new()
        .route("/api/project/todos/:id/executions",get(||async {Json(json!({"current_execution_id":"operator-current","assignments":[{"execution_id":"operator-current","kind":"operator"},{"execution_id":"agent-old","kind":"agent"},{"execution_id":"dag-old","kind":"dag"}]}))}))
        .route("/api/executions/:id/commands",post(|State(seen):State<Arc<Mutex<Vec<Value>>>>,Path(id):Path<String>,Json(body):Json<Value>|async move {seen.lock().unwrap().push(json!({"id":id,"body":body}));Json(json!({"accepted":true}))}))
        .route("/api/executions/:id/messages",get(|State(seen):State<Arc<Mutex<Vec<Value>>>>,Path(id):Path<String>|async move {seen.lock().unwrap().push(json!({"id":id,"read":"messages"}));Json(json!({"chunks":[],"more":false}))}))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    for args in [
        vec![
            "prompt",
            "todo",
            "--json",
            r#"{"prompt":"continue","input_id":"once","delivery":"queue"}"#,
        ],
        vec!["messages", "todo"],
        vec![
            "prompt",
            "todo",
            "--execution",
            "agent-old",
            "--json",
            r#"{"prompt":"history","input_id":"history-once"}"#,
        ],
    ] {
        let mut argv = vec![
            "opencoder-cli",
            "--server",
            &base,
            "--token",
            "test",
            "project",
            "todos",
        ];
        argv.extend(args);
        assert_eq!(
            opencoder_cli::run(opencoder_cli::Cli::try_parse_from(argv).unwrap())
                .await
                .unwrap(),
            0
        );
    }
    let argv = vec![
        "opencoder-cli",
        "--server",
        &base,
        "--token",
        "test",
        "project",
        "todos",
        "prompt",
        "todo",
        "--execution",
        "dag-old",
        "--json",
        r#"{"prompt":"invalid"}"#,
    ];
    assert_eq!(
        opencoder_cli::run(opencoder_cli::Cli::try_parse_from(argv).unwrap())
            .await
            .unwrap(),
        4
    );
    let rows = seen.lock().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        json!({"id":"operator-current","body":{"action":"queue","input":{"prompt":"continue","input_id":"once"}}})
    );
    assert_eq!(rows[1]["id"], "operator-current");
    assert_eq!(rows[2]["id"], "agent-old");
    task.abort();
}
