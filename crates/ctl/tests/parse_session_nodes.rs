//! Parsing + plan-mapping tests for the session and node domains
//! (rules/01: every subcommand has a case). Parsing goes through the real
//! `Cli` surface; plan assertions pin method, path (relay tail included),
//! query mapping and body passing — no network involved.

use clap::{CommandFactory, FromArgMatches};
use opencoder_cli::cmd::nodes::{plan as nodes_plan, NodesCmd};
use opencoder_cli::cmd::sessions::{plan as session_plan, SessionCmd};
use opencoder_cli::http::RequestPlan;
use opencoder_cli::Cli;
use serde_json::json;

/// Try parsing `opencoder-cli <args...>` into raw matches (Err on bad argv).
fn try_matches_of(args: &[&str]) -> Result<clap::ArgMatches, clap::Error> {
    let argv: Vec<String> = std::iter::once("opencoder-cli")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    Cli::command().try_get_matches_from(argv)
}

fn matches_of(args: &[&str]) -> clap::ArgMatches {
    try_matches_of(args).expect("valid cli invocation")
}

/// Parse `opencoder-cli <args...>` and unwrap the typed `session` subcommand.
fn session(args: &[&str]) -> SessionCmd {
    SessionCmd::from_arg_matches(matches_of(args).subcommand_matches("session").unwrap()).unwrap()
}

/// Parse `opencoder-cli <args...>` and unwrap the typed `nodes` subcommand.
fn nodes(args: &[&str]) -> NodesCmd {
    NodesCmd::from_arg_matches(matches_of(args).subcommand_matches("nodes").unwrap()).unwrap()
}

/// Assert a plan's full contract: method, path, query pairs, body.
#[allow(clippy::too_many_arguments)]
fn check(
    plan: RequestPlan,
    method: reqwest::Method,
    path: &str,
    query: &[(&str, &str)],
    body: Option<serde_json::Value>,
) {
    assert_eq!(plan.method, method, "method for {path}");
    assert_eq!(plan.path, path, "path");
    let want: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    assert_eq!(plan.query, want, "query for {path}");
    assert_eq!(plan.body, body, "body for {path}");
}

// ── sessions: parsing ─────────────────────────────────────────────────

#[test]
#[rustfmt::skip]
fn parses_session_subcommands() {
    assert!(matches!(session(&["session", "list"]), SessionCmd::List));
    assert!(matches!(session(&["session", "create", "--json", r#"{"agent":"act"}"#]), SessionCmd::Create { .. }));
    assert!(matches!(session(&["session", "get", "s1"]), SessionCmd::Get { id } if id == "s1"));
    assert!(matches!(session(&["session", "delete", "s1"]), SessionCmd::Delete { id } if id == "s1"));
    assert!(matches!(session(&["session", "messages", "s1"]), SessionCmd::Messages { id } if id == "s1"));
    assert!(matches!(session(&["session", "prompt", "s1", "--json", r#"{"prompt":"hi"}"#]), SessionCmd::Prompt { .. }));
    assert!(matches!(session(&["session", "events", "s1"]), SessionCmd::Events { after: 0, .. }));
    assert!(matches!(session(&["session", "events", "s1", "--after", "77"]), SessionCmd::Events { after: 77, .. }));
    assert!(matches!(session(&["session", "agent", "s1", "--json", r#"{"value":"plan"}"#]), SessionCmd::Agent { .. }));
    assert!(matches!(session(&["session", "model", "s1", "--json", r#"{"value":"x/y"}"#]), SessionCmd::Model { .. }));
    assert!(matches!(session(&["session", "interrupt", "s1"]), SessionCmd::Interrupt { .. }));
    assert!(matches!(session(&["session", "fork", "s1"]), SessionCmd::Fork { .. }));
    assert!(matches!(session(&["session", "compact", "s1"]), SessionCmd::Compact { .. }));
    assert!(matches!(session(&["session", "handoff", "s1"]), SessionCmd::Handoff { json: None, .. }));
    assert!(matches!(session(&["session", "skill", "s1", "--json", r#"{"skill":null}"#]), SessionCmd::Skill { .. }));
    assert!(matches!(session(&["session", "questions", "s1"]), SessionCmd::Questions { .. }));
    assert!(matches!(session(&["session", "answer", "s1", "q9", "--json", r#"{"answer":"y"}"#]), SessionCmd::Answer { qid, .. } if qid == "q9"));
    assert!(matches!(session(&["session", "skip", "s1", "q9"]), SessionCmd::Skip { qid, .. } if qid == "q9"));
    assert!(matches!(session(&["session", "inputs", "s1"]), SessionCmd::Inputs { delivery: None, .. }));
    assert!(matches!(session(&["session", "inputs", "s1", "--delivery", "queue"]), SessionCmd::Inputs { delivery: Some(d), .. } if d == "queue"));
    assert!(matches!(session(&["session", "input-reorder", "s1", "--json", r#"{"a":1,"b":2}"#]), SessionCmd::InputReorder { .. }));
    assert!(matches!(session(&["session", "input-delete", "s1", "42"]), SessionCmd::InputDelete { seq: 42, .. }));
    assert!(matches!(session(&["session", "annotation", "s1", "--json", r#"{"text":"t"}"#]), SessionCmd::Annotation { json: Some(_), .. }));
    assert!(matches!(session(&["session", "autopilot", "s1"]), SessionCmd::Autopilot { json: None, .. }));
    assert!(matches!(session(&["session", "subagents", "s1"]), SessionCmd::Subagents { .. }));
    assert!(matches!(session(&["session", "steer", "s1", "t7", "--json", r#"{"prompt":"go"}"#]), SessionCmd::Steer { task, .. } if task == "t7"));
    assert!(matches!(session(&["session", "task", "s1"]), SessionCmd::Task { id } if id == "s1"));
}

#[test]
fn session_required_json_body_is_enforced_by_clap() {
    for args in [
        &["session", "create"][..],
        &["session", "prompt", "s1"][..],
        &["session", "answer", "s1", "q9"][..],
        &["session", "input-reorder", "s1"][..],
        &["session", "steer", "s1", "t7"][..],
    ] {
        assert!(
            try_matches_of(args).is_err(),
            "expected parse failure for {args:?}"
        );
    }
}

// ── sessions: plan mapping ────────────────────────────────────────────

#[test]
#[rustfmt::skip]
fn session_plan_native_control_routes() {
    let get = reqwest::Method::GET;
    check(session_plan(&session(&["session", "list"])).unwrap(), get.clone(), "/api/sessions", &[], None);
    check(session_plan(&session(&["session", "create", "--json", r#"{"agent":"plan","model":"p/m"}"#])).unwrap(),
        reqwest::Method::POST, "/api/sessions", &[], Some(json!({"agent": "plan", "model": "p/m"})));
    // --after defaults to 0 (replay from the start).
    check(session_plan(&session(&["session", "events", "s1"])).unwrap(), get.clone(), "/api/sessions/s1/events", &[("after", "0")], None);
    check(session_plan(&session(&["session", "events", "s1", "--after", "9"])).unwrap(), get.clone(), "/api/sessions/s1/events", &[("after", "9")], None);
    check(session_plan(&session(&["session", "task", "s1"])).unwrap(), get, "/api/sessions/s1/task", &[], None);
}

#[test]
#[rustfmt::skip]
fn session_plan_relay_read_routes() {
    // Table of (argv, method, path): no query, no body on any of them.
    let cases: &[(&[&str], reqwest::Method, &str)] = &[
        (&["session", "get", "s1"], reqwest::Method::GET, "/api/sessions/s1"),
        // The web handler takes no query params for messages — none sent.
        (&["session", "messages", "s1"], reqwest::Method::GET, "/api/sessions/s1/messages"),
        (&["session", "questions", "s1"], reqwest::Method::GET, "/api/sessions/s1/questions"),
        (&["session", "subagents", "s1"], reqwest::Method::GET, "/api/sessions/s1/subagents"),
        (&["session", "delete", "s1"], reqwest::Method::DELETE, "/api/sessions/s1"),
        (&["session", "input-delete", "s1", "42"], reqwest::Method::DELETE, "/api/sessions/s1/inputs/42"),
    ];
    for (argv, method, path) in cases {
        check(session_plan(&session(argv)).unwrap(), method.clone(), path, &[], None);
    }
    // Inputs maps the delivery query; an absent flag sends no query pair.
    check(session_plan(&session(&["session", "inputs", "s1", "--delivery", "queue"])).unwrap(),
        reqwest::Method::GET, "/api/sessions/s1/inputs", &[("delivery", "queue")], None);
    check(session_plan(&session(&["session", "inputs", "s1"])).unwrap(), reqwest::Method::GET, "/api/sessions/s1/inputs", &[], None);
}

#[test]
#[rustfmt::skip]
fn session_plan_relay_posts_with_required_bodies() {
    let cases = [
        (vec!["session", "prompt", "s1", "--json", r#"{"prompt":"hi"}"#], "/api/sessions/s1/prompt", json!({"prompt": "hi"})),
        (vec!["session", "agent", "s1", "--json", r#"{"value":"plan"}"#], "/api/sessions/s1/agent", json!({"value": "plan"})),
        (vec!["session", "model", "s1", "--json", r#"{"value":"p/m"}"#], "/api/sessions/s1/model", json!({"value": "p/m"})),
        (vec!["session", "skill", "s1", "--json", r#"{"skill":"s"}"#], "/api/sessions/s1/skill", json!({"skill": "s"})),
        (vec!["session", "answer", "s1", "q9", "--json", r#"{"answer":"y"}"#], "/api/sessions/s1/questions/q9/answer", json!({"answer": "y"})),
        (vec!["session", "input-reorder", "s1", "--json", r#"{"a":1,"b":2}"#], "/api/sessions/s1/inputs/reorder", json!({"a": 1, "b": 2})),
        (vec!["session", "steer", "s1", "t7", "--json", r#"{"prompt":"go"}"#], "/api/sessions/s1/subagents/t7/steer", json!({"prompt": "go"})),
    ];
    for (argv, path, body) in cases {
        check(session_plan(&session(&argv)).unwrap(), reqwest::Method::POST, path, &[], Some(body));
    }
}

#[test]
#[rustfmt::skip]
fn session_plan_relay_posts_without_bodies() {
    let cases = [
        (vec!["session", "interrupt", "s1"], "/api/sessions/s1/interrupt"),
        (vec!["session", "fork", "s1"], "/api/sessions/s1/fork"),
        (vec!["session", "compact", "s1"], "/api/sessions/s1/compact"),
        (vec!["session", "skip", "s1", "q9"], "/api/sessions/s1/questions/q9/skip"),
    ];
    for (argv, path) in cases {
        check(session_plan(&session(&argv)).unwrap(), reqwest::Method::POST, path, &[], None);
    }
}

#[test]
#[rustfmt::skip]
fn session_plan_relay_optional_bodies() {
    // handoff/annotation/autopilot accept absent bodies (the server treats
    // them as defaults/clear) and forward a JSON body when one is given.
    for argv in [vec!["session", "handoff", "s1"], vec!["session", "annotation", "s1"], vec!["session", "autopilot", "s1"]] {
        assert_eq!(session_plan(&session(&argv)).unwrap().body, None, "{argv:?}");
    }
    let cases = [
        (vec!["session", "handoff", "s1", "--json", r#"{"extra":"e"}"#], "/api/sessions/s1/handoff", json!({"extra": "e"})),
        (vec!["session", "annotation", "s1", "--json", r#"{"text":"t"}"#], "/api/sessions/s1/annotation", json!({"text": "t"})),
        (vec!["session", "autopilot", "s1", "--json", r#"{"mode":"auto"}"#], "/api/sessions/s1/autopilot", json!({"mode": "auto"})),
    ];
    for (argv, path, body) in cases {
        check(session_plan(&session(&argv)).unwrap(), reqwest::Method::POST, path, &[], Some(body));
    }
}

#[test]
fn session_plan_rejects_invalid_json_bodies() {
    assert!(session_plan(&session(&["session", "create", "--json", "not json"])).is_err());
    assert!(session_plan(&session(&["session", "prompt", "s1", "--json", "oops"])).is_err());
    assert!(session_plan(&session(&["session", "steer", "s1", "t", "--json", "[bad"])).is_err());
}

// ── nodes: parsing + plan mapping ─────────────────────────────────────

#[test]
#[rustfmt::skip]
fn parses_nodes_subcommands() {
    assert!(matches!(nodes(&["nodes", "list"]), NodesCmd::List));
    assert!(matches!(nodes(&["nodes", "maintenance", "n1", "--json", r#"{"action":"gc"}"#]), NodesCmd::Maintenance { id, .. } if id == "n1"));
    assert!(matches!(nodes(&["nodes", "models"]), NodesCmd::Models { node: None }));
    assert!(matches!(nodes(&["nodes", "skills", "--node", "n1"]), NodesCmd::Skills { node: Some(n), .. } if n == "n1"));
    assert!(matches!(nodes(&["nodes", "dialogs", "n1"]), NodesCmd::Dialogs { node } if node == "n1"));
    assert!(matches!(nodes(&["nodes", "task-create", "n1", "--json", r#"{"prompt":"p"}"#]), NodesCmd::TaskCreate { .. }));
    assert!(matches!(nodes(&["nodes", "task-cancel", "n1", "t9"]), NodesCmd::TaskCancel { task, .. } if task == "t9"));
    assert!(matches!(nodes(&["nodes", "task-events", "t9"]), NodesCmd::TaskEvents { after: 0, .. }));
    assert!(matches!(nodes(&["nodes", "task-events", "t9", "--after", "5"]), NodesCmd::TaskEvents { after: 5, .. }));
}

#[test]
#[rustfmt::skip]
fn nodes_plan_routes() {
    let get = reqwest::Method::GET;
    check(nodes_plan(&nodes(&["nodes", "list"])).unwrap(), get.clone(), "/api/nodes", &[], None);
    check(nodes_plan(&nodes(&["nodes", "maintenance", "n1", "--json", r#"{"action":"gc"}"#])).unwrap(),
        reqwest::Method::POST, "/api/nodes/n1/maintenance", &[], Some(json!({"action": "gc"})));
    // models/skills: optional node_id query, absent by default.
    check(nodes_plan(&nodes(&["nodes", "models"])).unwrap(), get.clone(), "/api/models", &[], None);
    check(nodes_plan(&nodes(&["nodes", "skills", "--node", "n1"])).unwrap(), get.clone(), "/api/skills", &[("node_id", "n1")], None);
    check(nodes_plan(&nodes(&["nodes", "dialogs", "n1"])).unwrap(), get.clone(), "/api/nodes/n1/dialogs", &[], None);
    check(nodes_plan(&nodes(&["nodes", "task-create", "n1", "--json", r#"{"prompt":"p","agent":"act"}"#])).unwrap(),
        reqwest::Method::POST, "/api/nodes/n1/tasks", &[], Some(json!({"prompt": "p", "agent": "act"})));
    check(nodes_plan(&nodes(&["nodes", "task-cancel", "n1", "t9"])).unwrap(), reqwest::Method::POST, "/api/nodes/n1/tasks/t9/cancel", &[], None);
    check(nodes_plan(&nodes(&["nodes", "task-events", "t9"])).unwrap(), get.clone(), "/api/nodes/tasks/t9/events", &[("after", "0")], None);
    check(nodes_plan(&nodes(&["nodes", "task-events", "t9", "--after", "3"])).unwrap(), get, "/api/nodes/tasks/t9/events", &[("after", "3")], None);
}

#[test]
fn nodes_plan_rejects_invalid_json_bodies() {
    assert!(nodes_plan(&nodes(&["nodes", "maintenance", "n1", "--json", "nope"])).is_err());
    assert!(nodes_plan(&nodes(&["nodes", "task-create", "n1", "--json", "{oops"])).is_err());
}

#[test]
fn cli_accepts_global_flags_and_rejects_unknown_tails() {
    // Global flags (server/token/verbose) precede the domain subcommand;
    // parsing must succeed end-to-end without touching the network.
    assert!(try_matches_of(&[
        "--server",
        "http://127.0.0.1:8080",
        "--token",
        "t0",
        "--verbose",
        "session",
        "events",
        "s1",
        "--after",
        "2",
    ])
    .unwrap()
    .subcommand_matches("session")
    .is_some());
    // Unknown session tails are rejected at parse time by clap.
    assert!(try_matches_of(&["session", "no-such-tail", "s1"]).is_err());
}
