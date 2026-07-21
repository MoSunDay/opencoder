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
        "--workdir",
        "/tmp/proj",
        "--prompt-file",
        "persona.md",
    ]);
    assert_eq!(
        cli.workdir.as_deref(),
        Some(std::path::Path::new("/tmp/proj"))
    );
    assert_eq!(
        cli.prompt_file.as_deref(),
        Some(std::path::Path::new("persona.md"))
    );
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
fn server_subcommand_and_serve_alias() {
    // `serve` is kept as a backward-compat alias for `server`.
    let cli = parse(&[
        "opencoder",
        "serve",
        "--port",
        "9090",
        "--host",
        "127.0.0.1",
    ]);
    match cli.command {
        Some(Command::Server { port, host, .. }) => {
            assert_eq!(port, 9090);
            assert_eq!(host, "127.0.0.1");
        }
        _ => panic!("expected Server"),
    }

    // The canonical name works too, and accepts --token.
    let cli2 = parse(&[
        "opencoder",
        "server",
        "--port",
        "1",
        "--token",
        "abc",
    ]);
    match cli2.command {
        Some(Command::Server { token, .. }) => {
            assert_eq!(token.as_deref(), Some("abc"));
        }
        _ => panic!("expected Server"),
    }
}

#[test]
fn client_subcommand_parses() {
    use opencoder_cli::Command;
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://127.0.0.1:8080",
        "--token",
        "TKN",
        "do",
        "the thing",
    ]);
    match cli.command {
        Some(Command::Client {
            remote,
            token,
            session,
            continue_,
            prompt,
        }) => {
            assert_eq!(remote, "http://127.0.0.1:8080");
            assert_eq!(token.as_deref(), Some("TKN"));
            assert!(session.is_none());
            assert!(!continue_);
            assert_eq!(prompt, vec!["do".to_string(), "the thing".to_string()]);
        }
        _ => panic!("expected Client"),
    }

    // --session + --continue are accepted too
    let cli2 = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--session",
        "01ABC",
        "--continue",
        "hi",
    ]);
    match cli2.command {
        Some(Command::Client {
            session, continue_, ..
        }) => {
            assert_eq!(session.as_deref(), Some("01ABC"));
            assert!(continue_);
        }
        _ => panic!("expected Client"),
    }
}

#[test]
fn prompt_file_flag_parsed() {
    let cli = parse(&["opencoder", "--prompt-file", "x.md"]);
    assert_eq!(
        cli.prompt_file.as_deref(),
        Some(std::path::Path::new("x.md"))
    );
    // absent by default
    let cli2 = parse(&["opencoder"]);
    assert!(cli2.prompt_file.is_none());
}

#[test]
fn ts_subcommand_parses_list_flag() {
    let cli = parse(&["opencode", "ts", "-l"]);
    match cli.command {
        Some(Command::Ts { list, resume, new }) => {
            assert!(list);
            assert!(resume.is_none());
            assert!(!new);
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_parses_resume_target() {
    let cli = parse(&["opencode", "ts", "-r", "01HZ"]);
    match cli.command {
        Some(Command::Ts { list, resume, new }) => {
            assert!(!list);
            assert_eq!(resume.as_deref(), Some("01HZ"));
            assert!(!new);
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_parses_new_flag() {
    let cli = parse(&["opencode", "ts", "--new"]);
    match cli.command {
        Some(Command::Ts { list, resume, new }) => {
            assert!(!list);
            assert!(resume.is_none());
            assert!(new);
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_defaults_to_no_flags() {
    // Bare `opencode ts` -> Ts with every flag at its default.
    let cli = parse(&["opencode", "ts"]);
    match cli.command {
        Some(Command::Ts { list, resume, new }) => {
            assert!(!list);
            assert!(resume.is_none());
            assert!(!new);
        }
        _ => panic!("expected Ts"),
    }
}
