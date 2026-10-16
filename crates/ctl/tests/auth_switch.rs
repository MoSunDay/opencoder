//! Real CLI processes against the real control router in an isolated database.
use opencoder_core::identity::{token_hash, Role};
use opencoder_llm::MockChatClient;
use serde_json::{json, Value};
use std::sync::Arc;

const TOKEN: &str = "auth-switch-test-seed";

async fn call(config: &std::path::Path, args: &[&str]) -> (i32, Value) {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_opencoder-cli"))
        .arg("--client-config")
        .arg(config)
        .args(args)
        .env_remove("OPENCODER_SERVER_URL")
        .env_remove("OPENCODER_SERVER_TOKEN")
        .output()
        .await
        .unwrap();
    let code = output.status.code().unwrap();
    let body = serde_json::from_slice(if code == 0 {
        &output.stdout
    } else {
        &output.stderr
    })
    .unwrap_or_else(|_| panic!("stdout: {:?}, stderr: {:?}", output.stdout, output.stderr));
    (code, body)
}

async fn ok(config: &std::path::Path, args: &[&str]) -> Value {
    let (code, body) = call(config, args).await;
    assert_eq!(code, 0, "{args:?}: {body}");
    body
}

#[tokio::test]
async fn disabled_auth_supports_all_project_crud_and_preserves_existing_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(
        work.join("opencoder.json"),
        json!({"agent":{"agents_dir":dir.path().join("agents")}}).to_string(),
    )
    .unwrap();
    let state = opencoder_control::new_state(
        work,
        dir.path().join("data"),
        Some(Arc::new(MockChatClient::new())),
    )
    .await
    .unwrap();
    state
        .store
        .create_user("reader", &token_hash("reader-token"), Role::Viewer, 1)
        .await
        .unwrap();
    let users = state.store.list_users().await.unwrap();
    let tokens = state.store.list_access_tokens().await.unwrap();
    let mut tasks = vec![];
    let mut bases = vec![];
    for enabled in [false, true] {
        let app = opencoder_control::build_app_with_auth(
            state.clone(),
            Some(TOKEN.into()),
            Some("metrics-only".into()),
            false,
            enabled,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        bases.push(format!("http://{}", listener.local_addr().unwrap()));
        tasks.push(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
    }
    let config = dir.path().join("ctl.json");
    std::fs::write(
        &config,
        json!({"server":bases[0],"auth_enabled":false,
        "token_file":"intentionally-missing"})
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        ok(&config, &["raw", "call", "GET", "/api/auth/status"]).await,
        json!({"enabled":false})
    );
    assert_eq!(
        ok(&config, &["raw", "call", "GET", "/api/me"]).await,
        json!({"name":"admin","role":"admin"})
    );
    // Credentials are ignored while disabled, including a stored viewer token.
    assert_eq!(
        ok(
            &config,
            &["--token", "reader-token", "raw", "call", "GET", "/api/me"]
        )
        .await["role"],
        "admin"
    );
    assert_eq!(
        ok(&config, &["--token", "wrong", "health"]).await["ok"],
        true
    );

    let goal = ok(
        &config,
        &[
            "project",
            "goals",
            "create",
            "--json",
            r#"{"title":"CLI project","detail_md":"goal"}"#,
        ],
    )
    .await;
    let gid = goal["id"].as_str().unwrap();
    let group_body = json!({"goal_id":gid,"title":"CLI initiative"}).to_string();
    let group = ok(
        &config,
        &["project", "initiatives", "create", "--json", &group_body],
    )
    .await;
    let iid = group["id"].as_str().unwrap();
    let todo_file = dir.path().join("todo.json");
    std::fs::write(
        &todo_file,
        json!({"initiative_id":iid,"title":"CLI TODO","draft":"task"}).to_string(),
    )
    .unwrap();
    let todo = ok(
        &config,
        &[
            "project",
            "todos",
            "create",
            "--json",
            &format!("@{}", todo_file.display()),
        ],
    )
    .await;
    let tid = todo["id"].as_str().unwrap();
    for (kind, id, patch, key, expected) in [
        (
            "goals",
            gid,
            r#"{"title":"updated project"}"#,
            "title",
            "updated project",
        ),
        (
            "initiatives",
            iid,
            r#"{"detail_md":"updated initiative"}"#,
            "detail_md",
            "updated initiative",
        ),
        (
            "todos",
            tid,
            r#"{"draft":"updated task","board_status":"in_progress"}"#,
            "draft",
            "updated task",
        ),
    ] {
        ok(&config, &["project", kind, "patch", id, "--json", patch]).await;
        assert_eq!(
            ok(&config, &["project", kind, "get", id]).await[key],
            expected
        );
        let listed = ok(&config, &["project", kind, "list"]).await;
        assert_eq!(listed[kind].as_array().unwrap().len(), 1);
    }
    let note = ok(
        &config,
        &[
            "project",
            "todos",
            "comments",
            "add",
            tid,
            "--json",
            r#"{"request_id":"cli-check","body_md":"checked"}"#,
        ],
    )
    .await;
    assert_eq!(note["submitted_by"], "admin");
    assert_eq!(
        call(&config, &["project", "initiatives", "delete", iid])
            .await
            .0,
        4,
        "initiative with TODO must remain protected"
    );
    ok(&config, &["project", "goals", "delete", gid]).await;
    assert!(ok(&config, &["project", "initiatives", "get", iid]).await["goal_id"].is_null());
    ok(&config, &["project", "todos", "delete", tid]).await;
    ok(&config, &["project", "initiatives", "delete", iid]).await;
    for kind in ["goals", "initiatives", "todos"] {
        assert!(ok(&config, &["project", kind, "list"]).await[kind]
            .as_array()
            .unwrap()
            .is_empty());
    }
    // A token in defaults is also omitted when --no-auth is explicit.
    std::fs::write(
        &config,
        json!({"server":bases[1],"token":TOKEN}).to_string(),
    )
    .unwrap();
    assert_eq!(
        ok(
            &config,
            &["--no-auth", "raw", "call", "GET", "/api/auth/status"]
        )
        .await,
        json!({"enabled":true})
    );
    assert_eq!(call(&config, &["--no-auth", "health"]).await.0, 2);
    assert_eq!(ok(&config, &["health"]).await["ok"], true);
    assert_eq!(
        ok(
            &config,
            &["--token", "reader-token", "raw", "call", "GET", "/api/me"]
        )
        .await["role"],
        "viewer"
    );
    assert_eq!(
        call(
            &config,
            &[
                "--token",
                "reader-token",
                "project",
                "goals",
                "create",
                "--json",
                r#"{"title":"denied"}"#
            ]
        )
        .await
        .0,
        2
    );
    assert_eq!(state.store.list_users().await.unwrap(), users);
    assert_eq!(state.store.list_access_tokens().await.unwrap(), tokens);
    for task in tasks {
        task.abort();
    }
}

#[test]
fn anonymous_flags_are_explicit_and_conflict_with_credentials() {
    use clap::Parser;
    for credential in ["--token", "--token-file"] {
        assert!(opencoder_cli::Cli::try_parse_from([
            "cli",
            "--no-auth",
            credential,
            "value",
            "health"
        ])
        .is_err());
    }
}
