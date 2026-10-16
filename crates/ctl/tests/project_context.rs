//! TODO context follows its references, including history, native paging and errors.
use axum::{
    extract::{Path, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use opencoder_cli::{cmd::project::context, ctx::Ctx};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

type Seen = Arc<Mutex<Vec<String>>>;

async fn record(
    State(seen): State<Seen>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    assert_eq!(request.method(), "GET", "context must remain read-only");
    seen.lock().await.push(request.uri().to_string());
    next.run(request).await
}

async fn server() -> (Ctx, tokio::task::JoinHandle<()>, Seen) {
    let seen: Seen = Default::default();
    let router = Router::new()
        .route("/api/project/todos/:id", get(|Path(id): Path<String>| async move {
            if id == "missing" { (StatusCode::NOT_FOUND, Json(json!({"error":"todo not found"}))) }
            else if id == "denied" { (StatusCode::FORBIDDEN, Json(json!({"error":"forbidden"}))) }
            else { (StatusCode::OK, Json(json!({"id":id,"title":"核对问题","draft":"原始要求","board_status":"todo"}))) }
        }))
        .route("/api/project/todos/:id/files", get(|Path(id): Path<String>| async move {
            if id == "files-unavailable" { (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"file storage unavailable"}))) }
            else { (StatusCode::OK, Json(json!({"directory":"2026-01-01/t","files":[{"path":"notes.md","bytes":3}]}))) }
        }))
        .route("/api/project/todos/:id/executions", get(|Path(id): Path<String>| async move {
            Json(if id == "empty" { json!({"assignments":[],"current_execution_id":null}) }
            else { json!({"current_execution_id":null,"assignments":[
                {"execution_id":"history","kind":"dag"},
                {"execution_id":"current","kind":""},
                {"execution_id":"offline","kind":"dag"}
            ]}) })
        }))
        .route("/api/project/todos/:id/comments", get(|Query(q): Query<HashMap<String,String>>| async move {
            Json(if q.contains_key("before_seq") { json!({"comments":[{"seq":1,"body_md":"早期说明"}],"has_more":false}) }
            else { json!({"comments":[{"seq":2,"body_md":"最新补充"}],"has_more":true}) })
        }))
        .route("/api/executions/:id", get(|Path(id): Path<String>| async move {
            if id == "offline" { (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"original Runtime offline"}))) }
            else { (StatusCode::OK, Json(json!({"execution":{"id":id,"node_id":"original-runtime","kind":if id=="history" {"operator"} else {"agent"}},
                "request":{"input":{"prompt":"执行要求"}}, "session":{"meta":{"id":id}},
                "result":{"omitted":true,"read_via":"detail_field","field":"result"}}))) }
        }))
        .route("/api/executions/:id/:resource", get(|Path((id,resource)): Path<(String,String)>, Query(q): Query<HashMap<String,String>>| async move {
            if id == "offline" { return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"original Runtime offline"}))); }
            let value = match resource.as_str() {
                "result" => json!({"summary":"执行结论","truncated":false}),
                "events-page" => if q.contains_key("after") { json!({"events":[{"seq":2,"data":"后续过程"}],"more":false,"head_seq":2}) }
                    else { json!({"events":[{"seq":1,"data":{"omitted":true,"read_via":"event_payload"}}],"more":true,"head_seq":2}) },
                "messages" => {
                    let text = r#"[{"type":"text","text":"完整上下文"}]"#.as_bytes();
                    let offset = q["offset"].parse::<usize>().unwrap();
                    let end = if offset == 0 { text.len()-2 } else { text.len() };
                    json!({"chunks":[{"seq":1,"role":"assistant","offset":offset,"bytes_b64":STANDARD.encode(&text[offset..end])}],
                        "more":end<text.len(),"next_cursor":{"seq":1,"offset":end}})
                }
                "detail-field" => json!({"offset":0,"eof":true,"bytes_b64":STANDARD.encode(br#"{"output_text":"full result"}"#)}),
                _ => panic!("unexpected native resource {resource}"),
            };
            (StatusCode::OK, Json(value))
        }))
        .route("/api/executions/:id/events/:seq/payload", get(|| async {
            Json(json!({"offset":0,"eof":true,"bytes_b64":STANDARD.encode(br#"{"tool":"full process"}"#)}))
        }))
        .layer(middleware::from_fn_with_state(seen.clone(), record));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ctx = Ctx {
        server: format!("http://{}", listener.local_addr().unwrap()),
        token: None,
        verbose: false,
        http: opencoder_cli::http::client().unwrap(),
    };
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (ctx, task, seen)
}

#[tokio::test]
async fn context_reads_all_history_without_needing_a_current_capability_and_exposes_missing_evidence(
) {
    let (ctx, task, seen) = server().await;
    let data = context::collect(
        &ctx,
        &context::Options {
            id: "todo".into(),
            execution: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(data["record"]["board_status"], "todo");
    assert_eq!(data["record"]["comments"].as_array().unwrap().len(), 2);
    assert_eq!(data["record"]["comments"][0]["body_md"], "早期说明");
    assert_eq!(data["executions"].as_object().unwrap().len(), 3);
    let history = &data["executions"]["history"];
    assert_eq!(history["kind"], "operator");
    assert_eq!(data["executions"]["current"]["kind"], "agent");
    assert_eq!(history["detail"]["session"]["meta"]["id"], "history");
    assert_eq!(
        history["detail"]["execution"]["node_id"],
        "original-runtime"
    );
    assert_eq!(history["messages"][0]["content"][0]["text"], "完整上下文");
    assert_eq!(history["events"].as_array().unwrap().len(), 2);
    assert_eq!(history["events"][0]["data"]["tool"], "full process");
    assert_eq!(
        history["full_fields"]["result"]["output_text"],
        "full result"
    );
    assert_eq!(data["complete"], false);
    assert!(data["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["error"].as_str().unwrap().contains("Runtime offline")));
    assert!(seen
        .lock()
        .await
        .iter()
        .any(|path| path.contains("messages?seq=1&offset=")));
    task.abort();
}

#[tokio::test]
async fn context_selects_only_linked_history_and_keeps_empty_todos_readable() {
    let (ctx, task, seen) = server().await;
    let args = context::Options {
        id: "todo".into(),
        execution: Some("history".into()),
    };
    let data = context::collect(&ctx, &args).await.unwrap();
    assert_eq!(data["complete"], true);
    assert_eq!(data["selected_execution_id"], "history");
    assert_eq!(data["executions"].as_object().unwrap().len(), 1);
    assert_eq!(data["assignments"].as_array().unwrap().len(), 3);
    assert!(!seen
        .lock()
        .await
        .iter()
        .any(|path| path.starts_with("/api/executions/current")
            || path.starts_with("/api/executions/offline")));
    assert_eq!(
        context::run(
            &ctx,
            &context::Options {
                execution: Some("foreign".into()),
                ..args
            }
        )
        .await
        .unwrap(),
        4
    );
    let empty = context::collect(
        &ctx,
        &context::Options {
            id: "empty".into(),
            execution: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(empty["complete"], true);
    assert_eq!(empty["executions"], json!({}));
    task.abort();
}

#[tokio::test]
async fn context_cli_dispatches_prints_json_and_preserves_partial_and_http_exit_codes() {
    let (ctx, task, _) = server().await;
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("ctl.json");
    std::fs::write(&config, "{}").unwrap();
    for (id, selection, code) in [
        ("todo", Some("history"), 0),
        ("todo", None, 4),
        ("missing", None, 4),
        ("denied", None, 2),
    ] {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_opencoder-cli"));
        command.args([
            "--client-config",
            config.to_str().unwrap(),
            "--server",
            &ctx.server,
            "--no-auth",
            "project",
            "todos",
            "context",
            id,
        ]);
        if let Some(execution) = selection {
            command.args(["--execution", execution]);
        }
        let output = command.output().await.unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if id == "todo" {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["complete"], selection.is_some());
            assert_eq!(value["record"]["title"], "核对问题");
        } else {
            assert!(output.stdout.is_empty());
            let error: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(error["status"], if id == "missing" { 404 } else { 403 });
        }
    }
    task.abort();
}

#[tokio::test]
async fn context_reports_unavailable_task_files_without_claiming_complete_handoff() {
    let (ctx, task, _) = server().await;
    let value = context::collect(
        &ctx,
        &context::Options {
            id: "files-unavailable".into(),
            execution: Some("history".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(value["complete"], false);
    assert!(value["record"]["files"].is_null());
    assert!(value["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["source"].as_str().unwrap_or("").ends_with("/files")));
    task.abort();
}
