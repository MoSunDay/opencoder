//! Parse-level + plan-level coverage for the `exec` domain: every one of
//! the 13 subcommands must parse from real CLI argv and map onto the exact
//! method/path/query/body contract of the control plane's execution API.

use clap::{CommandFactory, FromArgMatches, Parser};
use opencoder_cli::cmd::executions::{plan, ExecCmd};
use opencoder_cli::http::RequestPlan;
use opencoder_cli::Cli;
use serde_json::json;

/// Parse `opencoder-cli <args...>` and unwrap the typed `exec` subcommand.
fn parse(args: &[&str]) -> ExecCmd {
    let argv: Vec<String> = std::iter::once("opencoder-cli")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    let matches = Cli::command()
        .try_get_matches_from(argv)
        .expect("valid cli invocation");
    let exec = matches.subcommand_matches("exec").expect("exec subcommand");
    ExecCmd::from_arg_matches(exec).expect("exec args map to ExecCmd")
}

fn check(
    sub: &ExecCmd,
    method: reqwest::Method,
    path: &str,
    query: &[(&str, &str)],
) -> RequestPlan {
    let mapped = plan(sub).expect("plan maps subcommand");
    assert_eq!(mapped.method, method, "method for {sub:?}");
    assert_eq!(mapped.path, path, "path for {sub:?}");
    let want: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    assert_eq!(mapped.query, want, "query for {sub:?}");
    mapped
}

#[test]
fn list_maps_filters_and_full_cursor() {
    let sub = parse(&[
        "exec",
        "list",
        "--node-id",
        "node-1",
        "--kind",
        "dag",
        "--limit",
        "7",
        "--cursor-created-at",
        "42",
        "--cursor-id",
        "01ABC",
    ]);
    check(
        &sub,
        reqwest::Method::GET,
        "/api/executions",
        &[
            ("node_id", "node-1"),
            ("kind", "dag"),
            ("limit", "7"),
            ("cursor_created_at", "42"),
            ("cursor_id", "01ABC"),
        ],
    );
    assert!(plan(&sub).unwrap().body.is_none());
}

#[test]
fn list_without_filters_is_bare_and_cursor_must_pair() {
    let sub = parse(&["exec", "list"]);
    check(&sub, reqwest::Method::GET, "/api/executions", &[]);
    // Half a cursor is rejected locally before touching the network.
    assert!(plan(&parse(&["exec", "list", "--cursor-created-at", "42"])).is_err());
    assert!(plan(&parse(&["exec", "list", "--cursor-id", "01ABC"])).is_err());
}

#[test]
fn create_posts_inline_json_body() {
    let sub = parse(&[
        "exec",
        "create",
        "--json",
        r#"{"id":"01X","kind":"agent","node_id":"node-1"}"#,
    ]);
    let mapped = check(&sub, reqwest::Method::POST, "/api/executions", &[]);
    assert_eq!(
        mapped.body,
        Some(json!({"id": "01X", "kind": "agent", "node_id": "node-1"}))
    );
}

#[test]
fn create_reads_body_from_at_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("body.json");
    std::fs::write(&file, r#"{"id":"01Y","kind":"dag","node_id":"node-2"}"#).unwrap();
    let arg = format!("@{}", file.display());
    let mapped = check(
        &parse(&["exec", "create", "--json", &arg]),
        reqwest::Method::POST,
        "/api/executions",
        &[],
    );
    assert_eq!(
        mapped.body,
        Some(json!({"id": "01Y", "kind": "dag", "node_id": "node-2"}))
    );
}

#[test]
fn get_reads_one_execution_by_id() {
    check(
        &parse(&["exec", "get", "01X"]),
        reqwest::Method::GET,
        "/api/executions/01X",
        &[],
    );
}

#[test]
fn cmd_wraps_action_and_optional_input() {
    // Alias `command` also dispatches; missing --json means input: null.
    let sub = parse(&["exec", "command", "01X", "--action", "cancel"]);
    let mapped = check(
        &sub,
        reqwest::Method::POST,
        "/api/executions/01X/commands",
        &[],
    );
    assert_eq!(
        mapped.body,
        Some(json!({"action": "cancel", "input": null}))
    );

    let sub = parse(&[
        "exec",
        "cmd",
        "01X",
        "--action",
        "steer",
        "--json",
        r#"{"text":"hi"}"#,
    ]);
    let mapped = check(
        &sub,
        reqwest::Method::POST,
        "/api/executions/01X/commands",
        &[],
    );
    assert_eq!(
        mapped.body,
        Some(json!({"action": "steer", "input": {"text": "hi"}}))
    );
}

#[test]
fn events_defaults_after_zero_and_accepts_override() {
    check(
        &parse(&["exec", "events", "01X"]),
        reqwest::Method::GET,
        "/api/executions/01X/events",
        &[("after", "0")],
    );
    check(
        &parse(&["exec", "events", "01X", "--after", "5"]),
        reqwest::Method::GET,
        "/api/executions/01X/events",
        &[("after", "5")],
    );
}

#[test]
fn events_page_uses_paged_route() {
    check(
        &parse(&["exec", "events-page", "01X", "--after", "9"]),
        reqwest::Method::GET,
        "/api/executions/01X/events-page",
        &[("after", "9")],
    );
}

#[test]
fn payload_nests_seq_in_path_and_offset_in_query() {
    check(
        &parse(&["exec", "payload", "01X", "3", "--offset", "128"]),
        reqwest::Method::GET,
        "/api/executions/01X/events/3/payload",
        &[("offset", "128")],
    );
    check(
        &parse(&["exec", "payload", "01X", "3"]),
        reqwest::Method::GET,
        "/api/executions/01X/events/3/payload",
        &[],
    );
}

#[test]
fn field_takes_required_field_and_optional_offset() {
    check(
        &parse(&["exec", "detail-field", "01X", "--field", "stdout"]),
        reqwest::Method::GET,
        "/api/executions/01X/detail-field",
        &[("field", "stdout")],
    );
    check(
        &parse(&[
            "exec", "field", "01X", "--field", "stderr", "--offset", "64",
        ]),
        reqwest::Method::GET,
        "/api/executions/01X/detail-field",
        &[("field", "stderr"), ("offset", "64")],
    );
}

#[test]
fn messages_cursor_is_optional_pair() {
    check(
        &parse(&["exec", "messages", "01X"]),
        reqwest::Method::GET,
        "/api/executions/01X/messages",
        &[],
    );
    check(
        &parse(&["exec", "messages", "01X", "--seq", "2", "--offset", "10"]),
        reqwest::Method::GET,
        "/api/executions/01X/messages",
        &[("seq", "2"), ("offset", "10")],
    );
}

#[test]
fn todo_items_use_after_ordinal_cursor() {
    check(
        &parse(&["exec", "todo-items", "01X", "--after-ordinal", "3"]),
        reqwest::Method::GET,
        "/api/executions/01X/todo-items",
        &[("after_ordinal", "3")],
    );
    check(
        &parse(&["exec", "todo-items", "01X"]),
        reqwest::Method::GET,
        "/api/executions/01X/todo-items",
        &[],
    );
}

#[test]
fn project_runs_use_before_version_cursor() {
    check(
        &parse(&["exec", "project-runs", "01X", "--before-version", "8"]),
        reqwest::Method::GET,
        "/api/executions/01X/project-runs",
        &[("before_version", "8")],
    );
}

#[test]
fn team_turns_use_after_turn_cursor() {
    check(
        &parse(&["exec", "team-turns", "01X", "--after-turn", "4"]),
        reqwest::Method::GET,
        "/api/executions/01X/team-turns",
        &[("after_turn", "4")],
    );
}

#[test]
fn artifact_defaults_and_explicit_output() {
    // Defaults: file omitted (server picks output.txt), out "-" = stdout.
    let sub = parse(&["exec", "artifact", "01X", "--step", "build"]);
    match &sub {
        ExecCmd::Artifact {
            out,
            file: None,
            step,
            ..
        } => {
            assert_eq!(step, "build");
            assert_eq!(out, std::path::Path::new("-"));
        }
        other => panic!("expected Artifact defaults, got {other:?}"),
    }
    check(
        &sub,
        reqwest::Method::GET,
        "/api/executions/01X/artifact",
        &[("step", "build")],
    );
    // Explicit file + destination path land in query / parsed flags.
    let sub = parse(&[
        "exec",
        "artifact",
        "01X",
        "--step",
        "build",
        "--file",
        "logs.txt",
        "-o",
        "/tmp/x.bin",
    ]);
    check(
        &sub,
        reqwest::Method::GET,
        "/api/executions/01X/artifact",
        &[("step", "build"), ("file", "logs.txt")],
    );
    match sub {
        ExecCmd::Artifact { out, .. } => assert_eq!(out, std::path::Path::new("/tmp/x.bin")),
        other => panic!("expected Artifact, got {other:?}"),
    }
}

#[test]
fn create_requires_json_flag() {
    assert!(Cli::try_parse_from(["opencoder-cli", "exec", "create"]).is_err());
}
