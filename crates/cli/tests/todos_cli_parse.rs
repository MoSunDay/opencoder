use clap::Parser;
use opencoder_cli::{Cli, Command, TodosSub};

#[test]
fn parses_todos_run_resume_and_debug_scope() {
    let cli = Cli::parse_from([
        "opencoder",
        "todos",
        "run",
        "--file",
        "case.json",
        "--debug",
    ]);
    match cli.command {
        Some(Command::Todos {
            sub:
                TodosSub::Run {
                    file,
                    workflow_id,
                    debug,
                    json,
                },
        }) => {
            assert_eq!(file.to_string_lossy(), "case.json");
            assert_eq!(workflow_id, None);
            assert!(debug);
            assert!(!json, "--json must default to false");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let cli = Cli::parse_from(["opencoder", "todos", "resume", "wf-1"]);
    assert!(matches!(
        cli.command,
        Some(Command::Todos {
            sub: TodosSub::Resume {
                debug: false,
                json: false,
                ..
            }
        })
    ));
}

#[test]
fn parses_caller_provided_todos_workflow_identity() {
    let cli = Cli::parse_from([
        "opencoder",
        "todos",
        "run",
        "--file",
        "case.json",
        "--workflow-id",
        "todos-canonical-42",
    ]);
    match cli.command {
        Some(Command::Todos {
            sub: TodosSub::Run { workflow_id, .. },
        }) => assert_eq!(workflow_id.as_deref(), Some("todos-canonical-42")),
        other => panic!("unexpected command: {other:?}"),
    }
    assert!(Cli::try_parse_from([
        "opencoder",
        "todos",
        "run",
        "--file",
        "case.json",
        "--workflow-id",
        "",
    ])
    .is_err());
}

#[test]
fn parses_todos_run_and_resume_json_flag() {
    let cli = Cli::parse_from(["opencoder", "todos", "run", "--file", "case.json", "--json"]);
    match cli.command {
        Some(Command::Todos {
            sub: TodosSub::Run { json, .. },
        }) => assert!(json),
        other => panic!("unexpected command: {other:?}"),
    }

    let cli = Cli::parse_from(["opencoder", "todos", "resume", "wf-1", "--json"]);
    match cli.command {
        Some(Command::Todos {
            sub: TodosSub::Resume { json, .. },
        }) => assert!(json),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn parses_todos_list_limit_with_default_100() {
    let cli = Cli::parse_from(["opencoder", "todos", "list", "--limit", "5"]);
    match cli.command {
        Some(Command::Todos {
            sub: TodosSub::List { limit, .. },
        }) => assert_eq!(limit, 5),
        other => panic!("unexpected command: {other:?}"),
    }

    let cli = Cli::parse_from(["opencoder", "todos", "list"]);
    match cli.command {
        Some(Command::Todos {
            sub: TodosSub::List { limit, .. },
        }) => assert_eq!(limit, 100),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn json_flag_is_not_a_global_or_validate_flag() {
    // `--json` is not a root flag: it falls through to the prompt text.
    let cli = Cli::parse_from(["opencoder", "--json", "todos", "list"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.prompt, vec!["--json", "todos", "list"]);
    assert!(Cli::try_parse_from([
        "opencoder",
        "todos",
        "validate",
        "--file",
        "case.json",
        "--json"
    ])
    .is_err());
    assert!(Cli::try_parse_from(["opencoder", "todos", "interrupt", "wf-1", "--json"]).is_err());
}

#[test]
fn show_and_events_keep_their_pre_existing_json_flag() {
    // `show`/`events` already expose `--json` (pre-existing behavior, kept
    // intact); run/resume/list gained their own flags in this change while
    // `validate`/`interrupt` stay flag-only-machine-readable-free.
    assert!(Cli::try_parse_from(["opencoder", "todos", "show", "wf-1", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["opencoder", "todos", "events", "wf-1", "--json"]).is_ok());
}

#[test]
fn debug_is_not_a_global_or_show_flag() {
    let cli = Cli::parse_from(["opencoder", "--debug", "todos", "list"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.prompt, vec!["--debug", "todos", "list"]);
    assert!(Cli::try_parse_from(["opencoder", "todos", "show", "wf-1", "--debug"]).is_err());
}

#[test]
fn parses_todos_validate_without_runtime_flags() {
    let cli = Cli::parse_from(["opencoder", "todos", "validate", "--file", "case.json"]);
    assert!(matches!(
        cli.command,
        Some(Command::Todos {
            sub: TodosSub::Validate { .. }
        })
    ));
}
