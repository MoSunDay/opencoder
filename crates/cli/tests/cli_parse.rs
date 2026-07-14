use clap::Parser;
use opencoder_cli::{Cli, Command, ConfigSub, SessionSub};

fn parse(args: &[&str]) -> Cli {
    Cli::parse_from(args)
}

#[test]
fn default_is_run_with_no_prompt() {
    let cli = parse(&["opencoder"]);
    assert!(cli.command.is_none());
    assert!(cli.prompt.is_empty());
    assert!(!cli.continue_);
    assert!(!cli.fork);
}

#[test]
fn global_flags_parsed() {
    let cli = parse(&[
        "opencoder",
        "--model",
        "glm-5.2",
        "--agent",
        "plan",
        "--small-model",
        "glm-flash",
    ]);
    assert_eq!(cli.model.as_deref(), Some("glm-5.2"));
    assert_eq!(cli.agent.as_deref(), Some("plan"));
    assert_eq!(cli.small_model.as_deref(), Some("glm-flash"));
}

#[test]
fn session_flag_sets_id() {
    let cli = parse(&["opencoder", "--session", "abc123"]);
    assert_eq!(cli.session.as_deref(), Some("abc123"));
}

#[test]
fn session_short_flag_sets_id() {
    let cli = parse(&["opencoder", "-s", "abc123"]);
    assert_eq!(cli.session.as_deref(), Some("abc123"));
}

#[test]
fn continue_and_fork_flags() {
    let cli = parse(&["opencoder", "--continue", "--fork"]);
    assert!(cli.continue_);
    assert!(cli.fork);
}

#[test]
fn tui_subcommand() {
    let cli = parse(&["opencoder", "tui"]);
    assert!(matches!(cli.command, Some(Command::Tui)));
}

#[test]
fn config_show_subcommand() {
    let cli = parse(&["opencoder", "config", "show"]);
    assert!(matches!(
        cli.command,
        Some(Command::Config {
            sub: Some(ConfigSub::Show)
        })
    ));
}

#[test]
fn session_subcommands() {
    let cli = parse(&["opencoder", "session", "list"]);
    assert!(matches!(
        cli.command,
        Some(Command::Session {
            sub: SessionSub::List
        })
    ));

    let cli = parse(&["opencoder", "session", "show", "sess-1"]);
    assert!(
        matches!(cli.command, Some(Command::Session { sub: SessionSub::Show { id, .. } }) if id == "sess-1")
    );

    let cli = parse(&["opencoder", "session", "show", "sess-1", "--json"]);
    assert!(matches!(
        cli.command,
        Some(Command::Session {
            sub: SessionSub::Show { id, json }
        }) if id == "sess-1" && json
    ));

    let cli = parse(&["opencoder", "session", "delete", "sess-2"]);
    assert!(
        matches!(cli.command, Some(Command::Session { sub: SessionSub::Delete { id } }) if id == "sess-2")
    );
}

#[test]
fn serve_subcommand() {
    let cli = parse(&[
        "opencoder",
        "serve",
        "--port",
        "9090",
        "--host",
        "127.0.0.1",
    ]);
    match cli.command {
        Some(Command::Serve { port, host, .. }) => {
            assert_eq!(port, 9090);
            assert_eq!(host, "127.0.0.1");
        }
        _ => panic!("expected Serve"),
    }
}
