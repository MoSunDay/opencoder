//! Parse-level + plan-mapping coverage for the `project`, `brain` and
//! `agents` domains (rules/01): every subcommand must survive clap parsing
//! and land on the verified method/path/query/body triple.

use clap::{CommandFactory, FromArgMatches};
use opencoder_cli::cmd::agents::{plan as plan_agents, AgentCmd};
use opencoder_cli::cmd::brain::{plan as plan_brain, BrainCmd};
use opencoder_cli::cmd::project::{plan as plan_project, ProjectCmd};
use opencoder_cli::http::RequestPlan;
use opencoder_cli::Cli;

/// Full-CLI match run against the real command tree (global flags included);
/// `Cli`'s fields are private, so extraction goes through clap's own
/// `FromArgMatches` impls instead of `Parser::try_parse_from`.
fn matches(args: &[&str]) -> clap::ArgMatches {
    let mut argv: Vec<&str> = vec!["opencoder-cli"];
    argv.extend_from_slice(args);
    Cli::command()
        .try_get_matches_from(argv)
        .expect("argv must parse")
}

/// The same tree must REJECT malformed argv (missing required flags, ...).
fn rejects(args: &[&str]) {
    let mut argv: Vec<&str> = vec!["opencoder-cli"];
    argv.extend_from_slice(args);
    assert!(
        Cli::command().try_get_matches_from(argv).is_err(),
        "argv must be rejected: {args:?}"
    );
}

fn project(args: &[&str]) -> ProjectCmd {
    let mut full = vec!["project"];
    full.extend_from_slice(args);
    ProjectCmd::from_arg_matches(
        matches(&full)
            .subcommand_matches("project")
            .expect("project subcommand"),
    )
    .expect("project args map to ProjectCmd")
}

fn brain(args: &[&str]) -> BrainCmd {
    let mut full = vec!["brain"];
    full.extend_from_slice(args);
    BrainCmd::from_arg_matches(
        matches(&full)
            .subcommand_matches("brain")
            .expect("brain subcommand"),
    )
    .expect("brain args map to BrainCmd")
}

fn agents(args: &[&str]) -> AgentCmd {
    let mut full = vec!["agents"];
    full.extend_from_slice(args);
    AgentCmd::from_arg_matches(
        matches(&full)
            .subcommand_matches("agents")
            .expect("agents subcommand"),
    )
    .expect("agents args map to AgentCmd")
}

fn planned_project(args: &[&str]) -> RequestPlan {
    plan_project(&project(args)).expect("plan")
}

fn planned_brain(args: &[&str]) -> RequestPlan {
    plan_brain(&brain(args)).expect("plan")
}

fn planned_agents(args: &[&str]) -> RequestPlan {
    plan_agents(&agents(args)).expect("plan")
}

// ── project ────────────────────────────────────────────────────────────

#[test]
fn project_overview_and_goal_crud() {
    let plan = planned_project(&["overview"]);
    assert_eq!(plan, RequestPlan::get("/api/project/overview"));

    let plan = planned_project(&["goals", "list"]);
    assert_eq!(plan, RequestPlan::get("/api/project/goals"));

    let plan = planned_project(&["goals", "create", "--json", r#"{"title":"ship"}"#]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/project/goals");
    assert_eq!(plan.body, Some(serde_json::json!({"title": "ship"})));

    let plan = planned_project(&["goals", "patch", "g1", "--json", r#"{"sort":2}"#]);
    assert_eq!(
        plan,
        RequestPlan::patch("/api/project/goals/g1").with_body(serde_json::json!({"sort": 2}))
    );

    let plan = planned_project(&["goals", "delete", "g1"]);
    assert_eq!(plan, RequestPlan::delete("/api/project/goals/g1"));
}

#[test]
fn project_milestones_filter_and_crud() {
    let plan = planned_project(&["milestones", "list"]);
    assert_eq!(plan, RequestPlan::get("/api/project/milestones"));
    assert!(plan.query.is_empty());

    let plan = planned_project(&["milestones", "list", "--goal", "g1"]);
    assert_eq!(plan.query, vec![("goal_id".to_owned(), "g1".to_owned())]);

    let plan = planned_project(&[
        "milestones",
        "create",
        "--json",
        r#"{"goal_id":"g1","title":"m"}"#,
    ]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/project/milestones");

    let plan = planned_project(&["milestones", "patch", "m1", "--json", r#"{"title":"x"}"#]);
    assert_eq!(plan.method, reqwest::Method::PATCH);
    assert_eq!(plan.path, "/api/project/milestones/m1");

    let plan = planned_project(&["milestones", "delete", "m1"]);
    assert_eq!(plan, RequestPlan::delete("/api/project/milestones/m1"));
}

#[test]
fn project_todos_lifecycle_runs_and_cancel() {
    let plan = planned_project(&["todos", "list"]);
    assert_eq!(plan, RequestPlan::get("/api/project/todos"));
    let plan = planned_project(&["todos", "list", "--milestone", "m1"]);
    assert_eq!(
        plan.query,
        vec![("milestone_id".to_owned(), "m1".to_owned())]
    );

    let plan = planned_project(&["todos", "create", "--json", r#"{"title":"t","draft":"d"}"#]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/project/todos");

    let plan = planned_project(&["todos", "patch", "t1", "--json", r#"{"title":"t2"}"#]);
    assert_eq!(plan.method, reqwest::Method::PATCH);
    assert_eq!(plan.path, "/api/project/todos/t1");

    let plan = planned_project(&["todos", "delete", "t1"]);
    assert_eq!(plan, RequestPlan::delete("/api/project/todos/t1"));

    // plan/execute take an OPTIONAL object body (may carry "run_id").
    let plan = planned_project(&["todos", "plan", "t1"]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/project/todos/t1/plan");
    assert!(plan.body.is_none());

    let plan = planned_project(&["todos", "execute", "t1", "--json", r#"{"run_id":"r1"}"#]);
    assert_eq!(plan.path, "/api/project/todos/t1/execute");
    assert_eq!(plan.body, Some(serde_json::json!({"run_id": "r1"})));

    let plan = planned_project(&["todos", "runs", "t1"]);
    assert_eq!(plan, RequestPlan::get("/api/project/todos/t1/runs"));
    let plan = planned_project(&["todos", "runs", "t1", "--before-version", "9"]);
    assert_eq!(
        plan.query,
        vec![("before_version".to_owned(), "9".to_owned())]
    );

    let plan = planned_project(&["todos", "cancel-run", "r1"]);
    assert_eq!(plan, RequestPlan::post("/api/project/runs/r1/cancel"));
    assert!(plan.body.is_none());
}

#[test]
fn project_body_accepts_at_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("todo.json");
    std::fs::write(&file, r#"{"title":"from file","draft":"d"}"#).unwrap();
    let json_at = format!("@{}", file.display());
    let plan = planned_project(&["todos", "create", "--json", json_at.as_str()]);
    assert_eq!(
        plan.body,
        Some(serde_json::json!({"title": "from file", "draft": "d"}))
    );
}

// ── brain ──────────────────────────────────────────────────────────────

#[test]
fn brain_caps_crud_and_target() {
    let plan = planned_brain(&["caps", "list"]);
    assert_eq!(plan, RequestPlan::get("/api/brain/capabilities"));

    let plan = planned_brain(&["caps", "create", "--json", r#"{"summary":"s"}"#]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/brain/capabilities");

    let plan = planned_brain(&["caps", "get", "c1"]);
    assert_eq!(plan, RequestPlan::get("/api/brain/capabilities/c1"));

    let plan = planned_brain(&["caps", "update", "c1", "--json", r#"{"summary":"s2"}"#]);
    assert_eq!(plan.method, reqwest::Method::PUT);
    assert_eq!(plan.path, "/api/brain/capabilities/c1");

    let plan = planned_brain(&["caps", "delete", "c1"]);
    assert_eq!(plan, RequestPlan::delete("/api/brain/capabilities/c1"));

    let plan = planned_brain(&["caps", "target-get", "c1"]);
    assert_eq!(plan, RequestPlan::get("/api/brain/capabilities/c1/target"));

    let plan = planned_brain(&[
        "caps",
        "target-bind",
        "c1",
        "--json",
        r#"{"capability_id":"c1"}"#,
    ]);
    assert_eq!(
        plan,
        RequestPlan::put("/api/brain/capabilities/c1/target")
            .with_body(serde_json::json!({"capability_id": "c1"}))
    );
}

#[test]
fn brain_search_plan_preview_dispatch() {
    let plan = planned_brain(&["search", "--json", r#"{"query":"auth","k":5}"#]);
    assert_eq!(
        plan,
        RequestPlan::post("/api/brain/search")
            .with_body(serde_json::json!({"query": "auth", "k": 5}))
    );

    let plan = planned_brain(&["plan", "--json", r#"{"situation":"deploy"}"#]);
    assert_eq!(
        plan,
        RequestPlan::post("/api/brain/plans").with_body(serde_json::json!({"situation": "deploy"}))
    );

    let plan = planned_brain(&["plan-get", "p1"]);
    assert_eq!(plan, RequestPlan::get("/api/brain/plans/p1"));

    let plan = planned_brain(&["preview", "--json", r#"{"situation":"deploy"}"#]);
    assert_eq!(
        plan,
        RequestPlan::post("/api/brain/preview")
            .with_body(serde_json::json!({"situation": "deploy"}))
    );

    let plan = planned_brain(&[
        "dispatch",
        "--json",
        r#"{"situation":"deploy","plan_id":"p1"}"#,
    ]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/brain/dispatch");
}

// ── agents ─────────────────────────────────────────────────────────────

#[test]
fn agents_cards_and_active_marker() {
    let plan = planned_agents(&["list"]);
    assert_eq!(plan, RequestPlan::get("/api/agents"));

    let plan = planned_agents(&["create", "--json", r#"{"name":"rev"}"#]);
    assert_eq!(
        plan,
        RequestPlan::post("/api/agents").with_body(serde_json::json!({"name": "rev"}))
    );

    let plan = planned_agents(&[
        "update",
        "rev",
        "--json",
        r#"{"current":{"prompt":"rev.md"}}"#,
    ]);
    assert_eq!(plan.method, reqwest::Method::PUT);
    assert_eq!(plan.path, "/api/agents/rev");

    let plan = planned_agents(&["delete", "rev"]);
    assert_eq!(plan, RequestPlan::delete("/api/agents/rev"));

    let plan = planned_agents(&["active", "--json", r#"{"active":"rev"}"#]);
    assert_eq!(
        plan,
        RequestPlan::patch("/api/agents/active").with_body(serde_json::json!({"active": "rev"}))
    );

    let plan = planned_agents(&["meta", "rev"]);
    assert_eq!(plan, RequestPlan::get("/api/agents/rev/meta"));
}

#[test]
fn agents_resources_crud_rollback_and_files() {
    let plan = planned_agents(&["resources", "list", "prompts"]);
    assert_eq!(plan, RequestPlan::get("/api/agents/resources/prompts"));

    let body = r#"{"name":"rev","files":[{"path":"rev.md","content_b64":"aGk="}]}"#;
    let plan = planned_agents(&["resources", "create", "prompts", "--json", body]);
    assert_eq!(plan.method, reqwest::Method::POST);
    assert_eq!(plan.path, "/api/agents/resources/prompts");
    assert!(plan.body.is_some());

    let plan = planned_agents(&["resources", "update", "prompts", "rev", "--json", body]);
    assert_eq!(plan.method, reqwest::Method::PUT);
    assert_eq!(plan.path, "/api/agents/resources/prompts/rev");

    let plan = planned_agents(&["resources", "delete", "prompts", "rev"]);
    assert_eq!(
        plan,
        RequestPlan::delete("/api/agents/resources/prompts/rev")
    );

    let plan = planned_agents(&["resources", "meta", "prompts", "rev"]);
    assert_eq!(
        plan,
        RequestPlan::get("/api/agents/resources/prompts/rev/meta")
    );

    let plan = planned_agents(&[
        "resources",
        "rollback",
        "prompts",
        "rev",
        "--json",
        r#"{"version":1}"#,
    ]);
    assert_eq!(
        plan,
        RequestPlan::post("/api/agents/resources/prompts/rev/rollback")
            .with_body(serde_json::json!({"version": 1}))
    );

    // Wildcard file path: positional segments join into one slash path.
    let plan = planned_agents(&[
        "resources",
        "file",
        "skills",
        "shell",
        "3",
        "python",
        "io.py",
    ]);
    assert_eq!(plan.method, reqwest::Method::GET);
    assert_eq!(
        plan.path,
        "/api/agents/resources/skills/shell/versions/3/files/python/io.py"
    );

    let plan = planned_agents(&["resources", "file", "prompts", "rev", "1", "rev.md"]);
    assert_eq!(
        plan.path,
        "/api/agents/resources/prompts/rev/versions/1/files/rev.md"
    );
}

#[test]
fn agents_nfs_status_and_set() {
    let plan = planned_agents(&["nfs", "status"]);
    assert_eq!(plan, RequestPlan::get("/api/agents/nfs"));

    let plan = planned_agents(&["nfs", "set", "--json", r#"{"enabled":true}"#]);
    assert_eq!(
        plan,
        RequestPlan::post("/api/agents/nfs").with_body(serde_json::json!({"enabled": true}))
    );
}

#[test]
fn malformed_bodies_fail_at_plan_time() {
    // clap rejects a missing required --json before plan() even runs.
    rejects(&["brain", "search"]);
    rejects(&["project", "goals", "create"]);
    // Invalid JSON bodies surface as plan() errors, never garbage sends.
    assert!(plan_brain(&brain(&["search", "--json", "not json"])).is_err());
    assert!(plan_agents(&agents(&["nfs", "set", "--json", "{oops}"])).is_err());
    // Resource file needs at least one path segment.
    rejects(&["agents", "resources", "file", "skills", "shell", "3"]);
}
