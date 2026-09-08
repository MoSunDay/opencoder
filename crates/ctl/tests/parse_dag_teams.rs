//! Parse-level contract tests for the `dag` / `teams` command
//! surfaces: every subcommand must map to the exact method + path + query +
//! body the control plane declares (crates/control/src/routes.rs and
//! api/compat/mod.rs). The list handlers expose no query extractors, so the
//! pinned plans below intentionally carry zero invented filters; the only
//! query anywhere is the SSE `after` cursor (defaults to 0).

use clap::Parser;
use opencoder_cli::cmd::dag::{plan as dag_plan, DagCmd};
use opencoder_cli::cmd::teams::{plan as teams_plan, TeamsCmd};
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
wrap!(dag, DagCmd);
wrap!(teams, TeamsCmd);

#[test]
fn dag_defs_crud() {
    assert_plan(
        &dag_plan(&dag(&["defs", "list"])).unwrap(),
        Method::GET,
        "/api/dag/defs",
        &[],
        None,
    );
    assert_plan(
        &dag_plan(&dag(&[
            "defs",
            "put",
            "--json",
            r#"{"spec":{"name":"etl"}}"#,
        ]))
        .unwrap(),
        Method::POST,
        "/api/dag/defs",
        &[],
        Some(json!({"spec": {"name": "etl"}})),
    );
    assert_plan(
        &dag_plan(&dag(&["defs", "get", "etl"])).unwrap(),
        Method::GET,
        "/api/dag/defs/etl",
        &[],
        None,
    );
    assert_plan(
        &dag_plan(&dag(&["defs", "delete", "etl"])).unwrap(),
        Method::DELETE,
        "/api/dag/defs/etl",
        &[],
        None,
    );
}

#[test]
fn dag_dispatch_body_is_optional_and_defaults_to_empty_object() {
    assert_plan(
        &dag_plan(&dag(&["dispatch", "etl", "--json", r#"{"node_id":"n1"}"#])).unwrap(),
        Method::POST,
        "/api/dag/defs/etl/dispatch",
        &[],
        Some(json!({"node_id": "n1"})),
    );
    assert_plan(
        &dag_plan(&dag(&["dispatch", "etl"])).unwrap(),
        Method::POST,
        "/api/dag/defs/etl/dispatch",
        &[],
        Some(json!({})),
    );
}

#[test]
fn dag_runs_inspect_and_cancel() {
    assert_plan(
        &dag_plan(&dag(&["runs", "list"])).unwrap(),
        Method::GET,
        "/api/dag/runs",
        &[],
        None,
    );
    assert_plan(
        &dag_plan(&dag(&["runs", "get", "01RUN"])).unwrap(),
        Method::GET,
        "/api/dag/runs/01RUN",
        &[],
        None,
    );
    assert_plan(
        &dag_plan(&dag(&["runs", "cancel", "01RUN"])).unwrap(),
        Method::POST,
        "/api/dag/runs/01RUN/cancel",
        &[],
        None,
    );
}

#[test]
fn dag_runs_events_after_cursor_defaults_to_zero() {
    assert_plan(
        &dag_plan(&dag(&["runs", "events", "01RUN"])).unwrap(),
        Method::GET,
        "/api/dag/runs/01RUN/events",
        &[("after", "0")],
        None,
    );
    assert_plan(
        &dag_plan(&dag(&["runs", "events", "01RUN", "--after", "42"])).unwrap(),
        Method::GET,
        "/api/dag/runs/01RUN/events",
        &[("after", "42")],
        None,
    );
}

#[test]
fn dag_rejects_malformed_bodies() {
    assert!(dag_plan(&dag(&["defs", "put", "--json", "not json"])).is_err());
    assert!(dag_plan(&dag(&["dispatch", "etl", "--json", "nope"])).is_err());
}

#[test]
fn teams_list_and_put() {
    assert_plan(
        &teams_plan(&teams(&["list"])).unwrap(),
        Method::GET,
        "/api/teams",
        &[],
        None,
    );
    assert_plan(
        &teams_plan(&teams(&["put", "--json", r#"{"name":"review"}"#])).unwrap(),
        Method::POST,
        "/api/teams",
        &[],
        Some(json!({"name": "review"})),
    );
    assert!(teams_plan(&teams(&["put", "--json", "oops"])).is_err());
}

#[test]
fn list_aliases_parse_to_the_same_surfaces() {
    assert!(matches!(dag(&["defs", "ls"]), DagCmd::Defs(_)));
    assert!(matches!(dag(&["runs", "ls"]), DagCmd::Runs(_)));
    assert!(matches!(teams(&["ls"]), TeamsCmd::List));
}
