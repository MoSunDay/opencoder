//! `todos list|show|events` open the store read-only, and a reader never
//! migrates: on a database older than this build the read fails, and the
//! command must say so with the remedy instead of surfacing the engine's raw
//! query error or blaming a missing workflow. A missing workflow on a current
//! schema keeps its own wording.
mod support;

use clap::Parser;
use opencoder_local::todos_cmd::dispatch;
use opencoder_local::{Cli, Command};
use opencoder_store::{Store, TursoStore};
use std::path::Path;
use std::sync::Arc;

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

/// The raw error this build's reader produces on the old schema, captured
/// straight from the store so the test does not hard-code engine wording.
async fn raw_reader_error(database: &Path) -> String {
    let reader: Arc<dyn Store> = Arc::new(TursoStore::open_reader(database).await.unwrap());
    opencoder_todos::persistence::list(&reader, 10)
        .await
        .expect_err("the old schema is not readable through a reader")
        .to_string()
}

/// Dispatch the parsed todos subcommand the way `main` does.
async fn dispatch_todos(cli: &Cli) -> anyhow::Result<()> {
    match cli.command.as_ref() {
        Some(Command::Todos { sub }) => dispatch(cli, sub).await,
        _ => panic!("expected a todos subcommand"),
    }
}

#[tokio::test]
async fn read_only_todos_commands_explain_an_old_schema_instead_of_the_raw_error() {
    let workdir = tempfile::tempdir().unwrap();
    let database = support::old_schema_store(workdir.path()).await;
    let raw = raw_reader_error(&database).await;

    for args in [vec!["list"], vec!["show", "OLD1"], vec!["events", "OLD1"]] {
        let cli = todos_cli(workdir.path(), &args);
        let error = dispatch_todos(&cli).await.unwrap_err();
        let top = error.to_string();
        let chain = format!("{error:#}");
        assert!(
            top.contains(&database.display().to_string())
                && top.contains("has schema version 3")
                && top.contains("read-only commands never migrate it")
                && top.contains("`opencoder tui`"),
            "`todos {}` must explain the old schema and its remedy, got: {chain}",
            args.join(" ")
        );
        assert!(
            !chain.contains("todo workflow not found"),
            "a store that cannot be read is not a missing workflow: {chain}"
        );
        assert!(
            chain.contains(&raw),
            "the engine's own error stays as the cause: {chain}"
        );
    }
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}

#[tokio::test]
async fn missing_workflow_on_a_current_schema_keeps_its_own_wording() {
    let workdir = tempfile::tempdir().unwrap();
    let database = support::database_for(workdir.path());
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    drop(TursoStore::open(&database).await.unwrap());

    for args in [vec!["show", "absent"], vec!["events", "absent"]] {
        let cli = todos_cli(workdir.path(), &args);
        let error = dispatch_todos(&cli).await.unwrap_err();
        let chain = format!("{error:#}");
        assert!(chain.contains("todo workflow not found: absent"), "{chain}");
        assert!(
            !chain.contains("older than this build"),
            "a current schema is never reported as old: {chain}"
        );
    }
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}
