use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::Parser;
use opencoder_cli::{cmd::project::summary, ctx::Ctx, Cli};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

type Seen = Arc<Mutex<Vec<String>>>;

#[test]
fn summary_version_commands_parse_and_target_the_matching_scope() {
    use opencoder_cli::cmd::project;
    #[derive(Parser)]
    struct Args {
        #[command(subcommand)]
        command: project::ProjectCmd,
    }
    for collection in ["goals", "initiatives"] {
        for (command, version, method, suffix) in [
            ("summary-versions", None, reqwest::Method::GET, ""),
            (
                "summary-restore",
                Some("7"),
                reqwest::Method::POST,
                "/7/restore",
            ),
        ] {
            let mut args = vec!["ctl", collection, command, "a/b"];
            args.extend(version);
            let command = Args::try_parse_from(args).unwrap().command;
            let plan = project::plan(&command).unwrap();
            assert_eq!(plan.method, method);
            assert_eq!(
                plan.path,
                format!("/api/project/{collection}/a%2Fb/summary-versions{suffix}")
            );
        }
        let command = Args::try_parse_from(["ctl", collection, "summary-restore", "p", "0"])
            .unwrap()
            .command;
        assert!(project::plan(&command).is_err());
    }
}

async fn server() -> (Ctx, tokio::task::JoinHandle<()>, Seen) {
    let seen: Seen = Default::default();
    let router = Router::new()
        .route("/api/executions/:id/events/:seq/payload", get(|| async { Json(json!({"offset":0,"eof":true,"bytes_b64":STANDARD.encode(br#"{"output":"full event"}"#)})) }))
        .route("/api/project/overview", get(|| async { Json(json!({
            "goals":[{"id":"p","title":"release","initiatives":[
                {"id":"i","current_summary":"implemented","summary_updated_at":1,"todos":[{"id":"a","board_status":"todo","updated_at":2},{"id":"b","board_status":"done","updated_at":1}]},
                {"id":"empty","todos":[]}
            ]}],"standalone_initiatives":[{"id":"standalone","todos":[]}]
        })) }))
        .route("/api/project/todos/:id/executions", get(|Path(id):Path<String>| async move {
            Json(if id == "a" { json!({"assignments":[{"execution_id":"old","kind":"operator"},{"execution_id":"offline","kind":"dag"},{"execution_id":"team","kind":"team"},{"execution_id":"workflow","kind":"todos"},{"execution_id":"brain","kind":"brain"}]}) }
                else { json!({"assignments":[{"execution_id":"old","kind":"operator"}]}) })
        }))
        .route("/api/project/todos/:id/comments", get(|Query(query):Query<HashMap<String,String>>| async move {
            Json(if query.contains_key("before_seq") { json!({"comments":[{"seq":1,"body_md":"first note"}],"has_more":false}) }
                else { json!({"comments":[{"seq":2,"body_md":"latest note"}],"has_more":true}) })
        }))
        .route("/api/executions/:id", get(|State(seen):State<Seen>,Path(id):Path<String>| async move {
            seen.lock().unwrap().push(id.clone());
            if id == "offline" { (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"node offline"}))) }
            else { (StatusCode::OK, Json(json!({"result":{"omitted":true,"field":"result","read_via":"detail_field"}}))) }
        }))
        .route("/api/executions/:id/:resource", get(|State(seen):State<Seen>, Path((id,resource)):Path<(String,String)>, Query(query):Query<HashMap<String,String>>| async move {
            seen.lock().unwrap().push(format!("{id}/{resource}:{query:?}"));
            let body = match resource.as_str() {
                "result" => json!({"summary":"latest","truncated":false}),
                "events-page" => if query.contains_key("after") { json!({"events":[{"seq":2,"data":"verified"}],"more":false,"head_seq":2}) }
                    else { json!({"events":[{"seq":1,"data":{"omitted":true,"read_via":"event_payload"}}],"more":true,"head_seq":if id == "brain" {1} else {2}}) },
                "messages" => {
                    let bytes = "接口完成".as_bytes();
                    let offset = query["offset"].parse::<usize>().unwrap();
                    let end = if offset == 0 { 2 } else { bytes.len() };
                    let mut page = json!({"chunks":[{"seq":1,"role":"assistant","offset":offset,"bytes_b64":STANDARD.encode(&bytes[offset..end]),"eof":end==bytes.len()}],"more":end<bytes.len()});
                    if end<bytes.len() { page["next_cursor"] = json!({"seq":1,"offset":end}); }
                    page
                }
                "team-turns" => if query.contains_key("after_turn") { json!({"turns":[{"turn":2}],"more":false}) } else { json!({"turns":[{"turn":1}],"more":true,"next_turn":1}) },
                "todo-items" => if query.contains_key("after_ordinal") { json!({"items":[{"ordinal":1}],"more":false}) } else { json!({"items":[{"ordinal":0}],"more":true,"next_ordinal":0}) },
                "detail-field" => json!({"offset":0,"eof":true,"bytes_b64":STANDARD.encode(br#"{"output_text":"full conclusion"}"#)}),
                _ => panic!("unexpected read: {resource}")
            };
            Json(body)
        }))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ctx = Ctx {
        server: format!("http://{}", listener.local_addr().unwrap()),
        token: Some("test".into()),
        verbose: false,
        http: opencoder_cli::http::client().unwrap(),
    };
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (ctx, task, seen)
}

#[tokio::test]
async fn initiative_reads_all_assignments_pages_full_messages_and_reports_offline_sources() {
    let (ctx, task, seen) = server().await;
    let data = summary::collect(&ctx, "initiative", "i").await.unwrap();
    assert_eq!(data["record"]["todos"].as_array().unwrap().len(), 2);
    assert_eq!(
        data["record"]["todos"][0]["assignments"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(data["record"]["todos"][0]["board_status"], "todo");
    assert_eq!(
        data["record"]["todos"][0]["comments"],
        json!([{"seq":1,"body_md":"first note"},{"seq":2,"body_md":"latest note"}])
    );
    assert_eq!(
        data["executions"]["brain"]["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        data["executions"]["team"]["turns"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(data["executions"]["workflow"]["items"][0]["ordinal"], 0);
    assert_eq!(
        data["executions"]["workflow"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        data["executions"]["old"]["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        data["executions"]["old"]["events"][0]["data"]["output"],
        "full event"
    );
    assert_eq!(
        data["executions"]["old"]["messages"][0]["content"],
        "接口完成"
    );
    assert_eq!(
        data["executions"]["old"]["full_fields"]["result"]["output_text"],
        "full conclusion"
    );
    assert!(data["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["error"].as_str().unwrap().contains("node offline")));
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|path| *path == "old")
            .count(),
        1
    );
    task.abort();
}

#[tokio::test]
async fn project_collects_every_initiative_and_warns_about_missing_or_outdated_summaries() {
    let (ctx, task, seen) = server().await;
    let data = summary::collect(&ctx, "project", "p").await.unwrap();
    assert_eq!(data["record"]["initiatives"].as_array().unwrap().len(), 2);
    assert_eq!(data["issues"].as_array().unwrap().len(), 2);
    assert_eq!(data["issues"][0]["initiative_id"], "i");
    assert_eq!(data["issues"][1]["initiative_id"], "empty");
    assert!(seen.lock().unwrap().is_empty());
    let empty = summary::collect(&ctx, "initiative", "standalone")
        .await
        .unwrap();
    assert_eq!(empty["record"]["todos"], json!([]));
    assert!(summary::collect(&ctx, "initiative", "missing")
        .await
        .is_err());
    task.abort();
}

#[tokio::test]
async fn summary_context_cli_parses_dispatches_and_keeps_http_exit_codes() {
    let (ctx, task, _) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("ctl.json");
    std::fs::write(&config, "{}").unwrap();
    for (scope, id, code) in [
        ("goals", "p", 0),
        ("initiatives", "standalone", 0),
        ("goals", "missing", 4),
    ] {
        let cli = Cli::try_parse_from([
            "opencoder-cli",
            "--client-config",
            config.to_str().unwrap(),
            "--server",
            &ctx.server,
            "--token",
            "test",
            "project",
            scope,
            "summary-context",
            id,
        ])
        .unwrap();
        assert_eq!(opencoder_cli::run(cli).await.unwrap(), code);
    }
    task.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let denied = Ctx {
        server: format!("http://{}", listener.local_addr().unwrap()),
        ..ctx
    };
    let router = Router::new().route(
        "/api/project/overview",
        get(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"unauthorized"})),
            )
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    assert_eq!(summary::run(&denied, "project", "p").await.unwrap(), 2);
    task.abort();
}

#[tokio::test]
async fn non_advancing_process_cursor_is_reported_without_hanging() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ctx = Ctx {
        server: format!("http://{}", listener.local_addr().unwrap()),
        token: Some("test".into()),
        verbose: false,
        http: opencoder_cli::http::client().unwrap(),
    };
    let router = Router::new()
        .route(
            "/api/project/overview",
            get(|| async {
                Json(json!({"goals":[],"standalone_initiatives":[{"id":"i","todos":[{"id":"t"}]}]}))
            }),
        )
        .route(
            "/api/project/todos/t/executions",
            get(|| async { Json(json!({"assignments":[{"execution_id":"run","kind":"dag"}]})) }),
        )
        .route(
            "/api/executions/run/events-page",
            get(|| async { Json(json!({"events":[{"seq":1}],"more":true})) }),
        )
        .fallback(get(|| async { Json(json!({})) }));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let data = summary::collect(&ctx, "initiative", "i").await.unwrap();
    assert!(data["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["error"].as_str().unwrap().contains("did not advance")));
    task.abort();
}
