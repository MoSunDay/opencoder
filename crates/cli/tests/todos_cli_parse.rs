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
            sub: TodosSub::Run { file, debug },
        }) => {
            assert_eq!(file.to_string_lossy(), "case.json");
            assert!(debug);
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let cli = Cli::parse_from(["opencoder", "todos", "resume", "wf-1"]);
    assert!(matches!(
        cli.command,
        Some(Command::Todos {
            sub: TodosSub::Resume { debug: false, .. }
        })
    ));
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
