//! Parse-level contract tests for the `todo` command
//! surfaces: every subcommand must map to the exact method + path + query +
//! body the control plane declares (crates/control/src/routes.rs and
//! api/compat/mod.rs). The list handlers expose no query extractors, so the
//! pinned plans below intentionally carry zero invented filters; the only
//! query anywhere is the SSE `after` cursor (defaults to 0).

use clap::Parser;
use opencoder_cli::cmd::todo::{plan as todo_plan, TodoCmd};
use opencoder_cli::http::RequestPlan;
use reqwest::Method;
use serde_json::json;

/// Assert the full request contract of a plan: method, path, ordered query
/// pairs and optional JSON body.
fn assert_plan(
    plan: &RequestPlan,
    method: Method,
    path: &str,
    query: &[(&str, &str)],
    body: Option<serde_json::Value>,
) {
    assert_eq!(plan.method, method, "method for {path}");
    assert_eq!(plan.path, path, "path");
    assert_eq!(
        plan.query,
        query
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>(),
        "query for {path}"
    );
    assert_eq!(plan.body, body, "body for {path}");
}

macro_rules! wrap {
    ($name:ident, $cmd:ty) => {
        fn $name(args: &[&str]) -> $cmd {
            #[derive(Parser)]
            struct Wrap {
                #[command(subcommand)]
                cmd: $cmd,
            }
            Wrap::try_parse_from(std::iter::once("cli").chain(args.iter().copied()))
                .unwrap()
                .cmd
        }
    };
}
wrap!(todo, TodoCmd);

#[test]
fn todo_envs_crud() {
    assert_plan(
        &todo_plan(&todo(&["envs", "list"])).unwrap(),
        Method::GET,
        "/api/todo/envs",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["envs", "put", "--json", r#"{"name":"gpu"}"#])).unwrap(),
        Method::POST,
        "/api/todo/envs",
        &[],
        Some(json!({"name": "gpu"})),
    );
    assert_plan(
        &todo_plan(&todo(&["envs", "get", "gpu"])).unwrap(),
        Method::GET,
        "/api/todo/envs/gpu",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&[
            "envs",
            "update",
            "gpu",
            "--json",
            r#"{"tools":[]}"#,
        ]))
        .unwrap(),
        Method::PUT,
        "/api/todo/envs/gpu",
        &[],
        Some(json!({"tools": []})),
    );
    assert_plan(
        &todo_plan(&todo(&["envs", "delete", "gpu"])).unwrap(),
        Method::DELETE,
        "/api/todo/envs/gpu",
        &[],
        None,
    );
}

#[test]
fn todo_tools_list_and_import() {
    assert_plan(
        &todo_plan(&todo(&["tools", "list"])).unwrap(),
        Method::GET,
        "/api/todo/tools",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&[
            "tools",
            "import",
            "--json",
            r#"{"agent":"ops","version":"v3","tool":"ffmpeg"}"#,
        ]))
        .unwrap(),
        Method::POST,
        "/api/todo/tools/import",
        &[],
        Some(json!({"agent": "ops", "version": "v3", "tool": "ffmpeg"})),
    );
}

#[test]
fn todo_templates_meta_and_delete() {
    assert_plan(
        &todo_plan(&todo(&["templates", "list"])).unwrap(),
        Method::GET,
        "/api/todo/templates",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&[
            "templates",
            "put",
            "--json",
            r#"{"name":"nightly"}"#,
        ]))
        .unwrap(),
        Method::POST,
        "/api/todo/templates",
        &[],
        Some(json!({"name": "nightly"})),
    );
    assert_plan(
        &todo_plan(&todo(&["templates", "get", "nightly"])).unwrap(),
        Method::GET,
        "/api/todo/templates/nightly",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["templates", "delete", "nightly"])).unwrap(),
        Method::DELETE,
        "/api/todo/templates/nightly",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["templates", "get-meta", "nightly"])).unwrap(),
        Method::GET,
        "/api/todo/templates/nightly/todo.json",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&[
            "templates",
            "put-meta",
            "nightly",
            "--json",
            r#"{"current":"v2"}"#,
        ]))
        .unwrap(),
        Method::PUT,
        "/api/todo/templates/nightly/todo.json",
        &[],
        Some(json!({"current": "v2"})),
    );
}

#[test]
fn todo_template_versions() {
    assert_plan(
        &todo_plan(&todo(&[
            "templates",
            "new-version",
            "nightly",
            "--json",
            r#"{"note":"bump"}"#,
        ]))
        .unwrap(),
        Method::POST,
        "/api/todo/templates/nightly/new-version",
        &[],
        Some(json!({"note": "bump"})),
    );
    assert_plan(
        &todo_plan(&todo(&["templates", "get-context", "nightly", "v2"])).unwrap(),
        Method::GET,
        "/api/todo/templates/nightly/v2/context.json",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&[
            "templates",
            "put-context",
            "nightly",
            "v2",
            "--json",
            r#"{"todos":[]}"#,
        ]))
        .unwrap(),
        Method::PUT,
        "/api/todo/templates/nightly/v2/context.json",
        &[],
        Some(json!({"todos": []})),
    );
    assert_plan(
        &todo_plan(&todo(&["templates", "get-binding", "nightly", "v2"])).unwrap(),
        Method::GET,
        "/api/todo/templates/nightly/v2/env.json",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&[
            "templates",
            "put-binding",
            "nightly",
            "v2",
            "--json",
            r#"{"env":"gpu"}"#,
        ]))
        .unwrap(),
        Method::PUT,
        "/api/todo/templates/nightly/v2/env.json",
        &[],
        Some(json!({"env": "gpu"})),
    );
    assert_plan(
        &todo_plan(&todo(&["templates", "delete-version", "nightly", "v2"])).unwrap(),
        Method::DELETE,
        "/api/todo/templates/nightly/v2",
        &[],
        None,
    );
}

#[test]
fn todo_run_body_is_optional_and_defaults_to_empty_object() {
    assert_plan(
        &todo_plan(&todo(&["run", "nightly", "v2"])).unwrap(),
        Method::POST,
        "/api/todo/templates/nightly/v2/run",
        &[],
        Some(json!({})),
    );
    assert_plan(
        &todo_plan(&todo(&[
            "run",
            "nightly",
            "v2",
            "--json",
            r#"{"id":"wf-1","node_id":"n1"}"#,
        ]))
        .unwrap(),
        Method::POST,
        "/api/todo/templates/nightly/v2/run",
        &[],
        Some(json!({"id": "wf-1", "node_id": "n1"})),
    );
}

#[test]
fn todo_workflows_control_and_events() {
    assert_plan(
        &todo_plan(&todo(&["workflows", "list"])).unwrap(),
        Method::GET,
        "/api/todo/workflows",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["workflows", "get", "01WF"])).unwrap(),
        Method::GET,
        "/api/todo/workflows/01WF",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["workflows", "interrupt", "01WF"])).unwrap(),
        Method::POST,
        "/api/todo/workflows/01WF/interrupt",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["workflows", "resume", "01WF"])).unwrap(),
        Method::POST,
        "/api/todo/workflows/01WF/resume",
        &[],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["workflows", "events", "01WF"])).unwrap(),
        Method::GET,
        "/api/todo/workflows/01WF/events",
        &[("after", "0")],
        None,
    );
    assert_plan(
        &todo_plan(&todo(&["workflows", "events", "01WF", "--after", "7"])).unwrap(),
        Method::GET,
        "/api/todo/workflows/01WF/events",
        &[("after", "7")],
        None,
    );
}

#[test]
fn list_aliases_parse_to_the_same_surfaces() {
    assert!(matches!(todo(&["envs", "ls"]), TodoCmd::Envs(_)));
    assert!(matches!(todo(&["workflows", "ls"]), TodoCmd::Workflows(_)));
}
