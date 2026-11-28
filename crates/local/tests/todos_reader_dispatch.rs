//! `todos list|show|events` open the store read-only: readers take no lease,
//! so they work beside any live writer (a `todos run` or a TUI in the same
//! workdir) without blocking or displacing it, report "nothing" without
//! creating the database, and leave the mutating subcommands on the writer
//! path that creates it. The multi-writer contention and maintenance-lease
//! suites live in the database crate's `tests/maintenance_lock.rs`,
//! `tests/busy_contention.rs` and `tests/multi_writer_soak.rs`.
use clap::Parser;
use opencoder_local::todos_cmd::dispatch;
use opencoder_local::{Cli, Command};
use opencoder_store::{Store, TursoStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const WORKFLOW: &str = "todos-beside";

fn todos_cli(workdir: &Path, args: &[&str]) -> Cli {
    let mut argv: Vec<String> = vec![
        "opencoder".into(),
        "--workdir".into(),
        workdir.to_string_lossy().into_owned(),
        "todos".into(),
    ];
    argv.extend(args.iter().map(|arg| (*arg).to_string()));
    Cli::parse_from(argv)
}

async fn dispatch_todos(cli: &Cli, args: &[&str]) -> anyhow::Result<()> {
    let Some(Command::Todos { sub }) = cli.command.as_ref() else {
        panic!(
            "`todos {}` did not parse into a todos subcommand",
            args.join(" ")
        );
    };
    dispatch(cli, sub).await
}

/// The per-workdir database `todos` resolves for `workdir`.
fn database_for(workdir: &Path) -> PathBuf {
    opencoder_core::data_dir_for(workdir).join("opencoder.db")
}

/// Seed the exact store the CLI opens for `workdir` with one workflow and
/// two events, and return the live writer the way a `todos run` owns it.
async fn seed_with_live_writer(workdir: &Path) -> Arc<dyn Store> {
    let data_dir = opencoder_core::data_dir_for(workdir);
    tokio::fs::create_dir_all(&data_dir).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(
        TursoStore::open(data_dir.join("opencoder.db"))
            .await
            .unwrap(),
    );
    let spec = {
        let raw = serde_json::json!({
            "schema_version": 1,
            "id": WORKFLOW,
            "name": "beside a live writer",
            "objective": "exercise the read-only todos commands",
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
        });
        opencoder_todos::parse_spec(&raw.to_string()).expect("spec parses")
    };
    let mut state =
        opencoder_todos::domain::initial_state(&spec, WORKFLOW.into(), "parent-beside".into());
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
    assert_eq!(store.todo_events_after(WORKFLOW, 0).await.unwrap().len(), 3);
    store
}

#[tokio::test]
async fn read_only_todos_commands_work_beside_a_live_writer() {
    let workdir = tempfile::tempdir().unwrap();
    let database = database_for(workdir.path());
    let writer = seed_with_live_writer(workdir.path()).await;

    for args in [
        vec!["list"],
        vec!["list", "--json"],
        vec!["show", WORKFLOW],
        vec!["show", WORKFLOW, "--json"],
        vec!["events", WORKFLOW],
        vec!["events", WORKFLOW, "--after", "1", "--json"],
    ] {
        let cli = todos_cli(workdir.path(), &args);
        dispatch_todos(&cli, &args).await.unwrap_or_else(|error| {
            panic!(
                "`todos {}` must succeed beside the live writer: {error:#}",
                args.join(" ")
            )
        });
    }

    // The writer is undisturbed: it still has the database open and keeps writing.
    let (spec, mut state) = opencoder_todos::persistence::load(&writer, WORKFLOW)
        .await
        .unwrap()
        .expect("the seeded workflow");
    state.generation += 1;
    opencoder_todos::persistence::commit(&writer, &spec, &state, "marker", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(
        writer.todo_events_after(WORKFLOW, 0).await.unwrap().len(),
        4,
        "the read-only commands changed nothing and blocked nothing"
    );
    drop(writer);
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}

#[tokio::test]
async fn read_only_todos_commands_report_nothing_without_creating_a_database() {
    let workdir = tempfile::tempdir().unwrap();
    let database = database_for(workdir.path());
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
    assert!(!database.exists());

    for args in [vec!["list"], vec!["list", "--json"]] {
        let cli = todos_cli(workdir.path(), &args);
        dispatch_todos(&cli, &args).await.unwrap_or_else(|error| {
            panic!("`todos {}` over no database: {error:#}", args.join(" "))
        });
    }
    for sub in ["show", "events"] {
        let args = [sub, "absent"];
        let cli = todos_cli(workdir.path(), &args);
        let error = dispatch_todos(&cli, &args).await.unwrap_err();
        let chain = format!("{error:#}");
        assert!(
            chain.contains("todo workflow not found: absent"),
            "`todos {sub}` must name the workflow id, got: {chain}"
        );
    }
    assert!(
        !database.exists(),
        "a read-only command must never create the database"
    );

    // `todos interrupt` is a reader as well: it never opens the store, so
    // over no database it names the workflow id and creates nothing.
    let args = ["interrupt", "absent"];
    let cli = todos_cli(workdir.path(), &args);
    let error = dispatch_todos(&cli, &args).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("absent"),
        "interrupt error must name the workflow id: {error:#}"
    );
    assert!(
        !database.exists(),
        "interrupt must not open the store for writing either"
    );
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}
