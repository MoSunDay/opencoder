//! Dispatch-level integration tests for the `todos` CLI surface.
//!
//! Everything here drives `opencoder_cli::todos_cmd::dispatch` through the
//! public API with `--workdir` pointed at a unique temp dir. dispatch opens
//! its own store under `<data_local>/opencoder/<hash(workdir)>`, so a fresh
//! temp workdir guarantees an empty per-workdir DB (no repo-state coupling).
//! Only paths that never construct a model client are covered here; the
//! run→resume happy path needs a live model and lives in the python e2e suite.

use clap::Parser;
use opencoder_cli::todos_cmd::{
    dispatch, render_final_state, todos_terminal_outcome, TodosOutcome,
};
use opencoder_cli::{Cli, Command};
use opencoder_todos::WorkflowState;

/// Parse `opencoder --workdir <dir> todos <args...>` into a Cli (the todos
/// subcommand is asserted by `dispatch_todos`).
fn todos_cli(workdir: &std::path::Path, args: &[&str]) -> Cli {
    let mut argv: Vec<String> = vec![
        "opencoder".into(),
        "--workdir".into(),
        workdir.to_string_lossy().into_owned(),
        "todos".into(),
    ];
    argv.extend(args.iter().map(|a| (*a).to_string()));
    Cli::parse_from(argv)
}

/// Dispatch the parsed Cli against its todos subcommand (borrows both).
async fn dispatch_todos(cli: &Cli) -> anyhow::Result<()> {
    let Some(Command::Todos { sub }) = cli.command.as_ref() else {
        panic!(
            "args did not parse into a todos subcommand: {:?}",
            cli.command
        );
    };
    dispatch(cli, sub).await
}

/// Full anyhow error chain ("outer: inner") so context lines are visible.
fn chain(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

/// A minimal valid spec: one todo with every required field populated.
fn good_spec_json() -> String {
    serde_json::json!({
        "schema_version": 1,
        "id": "wf-dispatch-test",
        "name": "dispatch test",
        "objective": "exercise the todos CLI dispatch surface",
        "constraints": [],
        "todos": [{
            "id": "t1",
            "title": "single todo",
            "requirement_background": "background for the todo",
            "instructions": "create a file and verify it",
            "depends_on": [],
            "agent": "act",
            "max_attempts": 1,
            "acceptance": {
                "criteria": "the file exists",
                "required_tool_calls": []
            }
        }],
        "metadata": {}
    })
    .to_string()
}

/// A small WorkflowState literal mirroring crates/todos/src/types.rs fields
/// (empty todos map keeps it minimal but fully typed on deserialize).
fn state_from(status: &str, terminal_reason: Option<&str>) -> WorkflowState {
    let mut raw = serde_json::json!({
        "workflow_id": "todos-01TEST",
        "parent_session_id": "s-01TEST",
        "status": status,
        "generation": 2,
        "world_epoch": 1,
        "active_todo_ids": [],
        "todos": {},
        "milestones": [],
        "incidents": []
    });
    if let Some(reason) = terminal_reason {
        raw["terminal_reason"] = serde_json::Value::String(reason.into());
    }
    serde_json::from_value(raw).expect("literal WorkflowState must deserialize")
}

#[tokio::test]
async fn events_unknown_id_errors_with_exit_context() {
    let dir = tempfile::tempdir().unwrap();
    let cli = todos_cli(dir.path(), &["events", "nope-1", "--json"]);
    let err = dispatch_todos(&cli).await.unwrap_err();
    let chain = chain(&err);
    assert!(
        chain.contains("todo workflow not found: nope-1"),
        "events error must name the workflow id, got: {chain}"
    );
}

#[tokio::test]
async fn show_unknown_id_errors() {
    // Guards the aligned `todo workflow not found: {id}` contract for show.
    let dir = tempfile::tempdir().unwrap();
    let cli = todos_cli(dir.path(), &["show", "nope-1"]);
    let err = dispatch_todos(&cli).await.unwrap_err();
    let chain = chain(&err);
    assert!(
        chain.contains("todo workflow not found: nope-1"),
        "show error must name the workflow id, got: {chain}"
    );
}

#[tokio::test]
async fn list_on_empty_workdir_outputs_nothing_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let cli = todos_cli(dir.path(), &["list"]);
    assert!(matches!(
        cli.command,
        Some(opencoder_cli::Command::Todos {
            sub: opencoder_cli::TodosSub::List {
                json: false,
                limit: 100
            }
        })
    ));
    dispatch_todos(&cli)
        .await
        .expect("list over an empty workdir must succeed");
}

#[tokio::test]
async fn validate_reports_file_path_on_bad_spec() {
    let dir = tempfile::tempdir().unwrap();
    let spec_path = dir.path().join("bad.json");
    std::fs::write(&spec_path, "{ this is not json").unwrap();
    // --file is resolved by dispatch relative to the process CWD, so tests
    // pass an absolute path; the error message still names the file's leaf.
    let cli = todos_cli(
        dir.path(),
        &["validate", "--file", spec_path.to_str().unwrap()],
    );
    let err = dispatch_todos(&cli).await.unwrap_err();
    let chain = chain(&err);
    assert!(
        chain.contains("bad.json"),
        "spec parse error must include the file path, got: {chain}"
    );
}

#[tokio::test]
async fn validate_accepts_good_spec() {
    let dir = tempfile::tempdir().unwrap();
    let spec_path = dir.path().join("good.json");
    std::fs::write(&spec_path, good_spec_json()).unwrap();
    let cli = todos_cli(
        dir.path(),
        &["validate", "--file", spec_path.to_str().unwrap()],
    );
    dispatch_todos(&cli)
        .await
        .expect("a schema-complete single-todo spec must validate");
}

#[test]
fn render_final_state_pretty_vs_json() {
    let state = state_from("suspended", Some("local interrupt requested"));
    let compact = render_final_state(&state, true);
    let pretty = render_final_state(&state, false);

    assert!(!compact.contains('\n'), "compact mode must be single-line");
    assert!(pretty.contains('\n'), "pretty mode must be multi-line");

    let compact_doc: serde_json::Value = serde_json::from_str(&compact).unwrap();
    let pretty_doc: serde_json::Value = serde_json::from_str(&pretty).unwrap();
    assert_eq!(compact_doc["status"], "suspended");
    assert_eq!(pretty_doc["status"], "suspended");
    assert_eq!(
        compact_doc, pretty_doc,
        "both modes must carry the same document"
    );

    // stdout contract: ONLY the JSON document — no workflow_id= prefix text.
    assert!(!compact.contains("workflow_id="));
    assert!(!pretty.contains("workflow_id="));
}

#[test]
fn terminal_outcome_maps_completed_interrupted_and_ended() {
    let completed = state_from("completed", None);
    assert_eq!(todos_terminal_outcome(&completed), TodosOutcome::Completed);

    let interrupted = state_from("suspended", Some("local interrupt requested"));
    assert_eq!(
        todos_terminal_outcome(&interrupted),
        TodosOutcome::Interrupted
    );

    let failed = state_from("failed", Some("todo exhausted attempts"));
    assert_eq!(
        todos_terminal_outcome(&failed),
        TodosOutcome::Ended("failed")
    );

    // Suspended under a DIFFERENT reason (e.g. the doom-loop guard) is a
    // generic terminal end, NOT a user Ctrl-C: it must not map to Interrupted.
    let guard = state_from("suspended", Some("doom loop guard"));
    assert_eq!(
        todos_terminal_outcome(&guard),
        TodosOutcome::Ended("suspended")
    );
}

#[tokio::test]
async fn interrupt_unknown_workflow_errors_with_id() {
    let dir = tempfile::tempdir().unwrap();
    let cli = todos_cli(dir.path(), &["interrupt", "nope-2"]);
    let err = dispatch_todos(&cli).await.unwrap_err();
    let chain = chain(&err);
    assert!(
        chain.contains("nope-2"),
        "interrupt error must name the workflow id, got: {chain}"
    );
}

/// `events --after <seq>` cursor contract (store-backed, key-free): after the
/// latest seq nothing remains, after last-1 exactly the newest event. The
/// CLI passes the cursor through to `todo_events_after`; content assertions
/// read the same DB the command opened.
#[tokio::test]
async fn events_after_cursor_sees_only_newer_events() {
    let workdir = tempfile::tempdir().unwrap();
    // Seed the exact store the CLI opens for this workdir.
    let data_dir = opencoder_core::data_dir_for(workdir.path());
    tokio::fs::create_dir_all(&data_dir).await.unwrap();
    let store: std::sync::Arc<dyn opencoder_store::Store> = std::sync::Arc::new(
        opencoder_store::LibsqlStore::open(data_dir.join("opencoder.db"))
            .await
            .unwrap(),
    );
    let spec = opencoder_todos::parse_spec(&good_spec_json()).expect("spec parses");
    let mut state =
        opencoder_todos::domain::initial_state(&spec, "todos-cursor".into(), "parent".into());
    opencoder_todos::parent::create_session(&store, &state, &opencoder_core::Config::default())
        .await
        .unwrap();
    opencoder_todos::persistence::create(&store, &spec, &state)
        .await
        .unwrap();
    for kind in ["workflow_started", "marker"] {
        state.generation += 1;
        opencoder_todos::persistence::commit(&store, &spec, &state, kind, serde_json::json!({}))
            .await
            .unwrap();
    }
    let events = store.todo_events_after("todos-cursor", 0).await.unwrap();
    assert_eq!(events.len(), 3);
    let last = events.last().unwrap().seq.unwrap();

    // Store-level cursor semantics with non-zero cursors.
    assert!(store
        .todo_events_after("todos-cursor", last)
        .await
        .unwrap()
        .is_empty());
    let tail = store
        .todo_events_after("todos-cursor", last - 1)
        .await
        .unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].seq, Some(last));

    // CLI wiring: both cursors parse, dispatch and exit 0.
    for after in [last, last - 1] {
        let cli = todos_cli(
            workdir.path(),
            &[
                "events",
                "todos-cursor",
                "--after",
                &after.to_string(),
                "--json",
            ],
        );
        dispatch_todos(&cli)
            .await
            .unwrap_or_else(|error| panic!("events --after {after} must succeed: {error:#}"));
    }
}

#[tokio::test]
async fn run_with_unresolvable_config_fails_cleanly() {
    // Isolate config discovery + env overlays to a temp home (thread-local,
    // so parallel tests are unaffected): no provider/api_key anywhere means
    // runtime() must fail at endpoint resolution BEFORE any client is built
    // or any model call attempted — an Err, never a hang or a panic.
    let dir = tempfile::tempdir().unwrap();
    let spec_path = dir.path().join("case.json");
    std::fs::write(&spec_path, good_spec_json()).unwrap();
    let _guard = opencoder_core::scoped_config_home(dir.path().join("cfg-home"));
    let cli = todos_cli(
        dir.path(),
        &["run", "--file", spec_path.to_str().unwrap(), "--json"],
    );
    let err = dispatch_todos(&cli)
        .await
        .expect_err("unresolvable endpoint config must fail the run");
    // The spec must have been read fine (absolute path) — the failure has to
    // come from endpoint resolution, not a missing file.
    let chain = chain(&err);
    assert!(
        !chain.contains("read todos file"),
        "spec read must succeed; got: {chain}"
    );
}
