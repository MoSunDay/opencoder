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
fn config_set_subcommand_parsed() {
    let cli = parse(&["opencoder", "config", "set", "anthropic/claude-3"]);
    match cli.command {
        Some(Command::Config {
            sub: Some(ConfigSub::Set { model }),
        }) => {
            assert_eq!(model, "anthropic/claude-3");
        }
        other => panic!("expected ConfigSub::Set, got {other:?}"),
    }
}

#[test]
fn config_set_bare_model_parsed() {
    let cli = parse(&["opencoder", "config", "set", "glm-5.2"]);
    match cli.command {
        Some(Command::Config {
            sub: Some(ConfigSub::Set { model }),
        }) => {
            assert_eq!(model, "glm-5.2");
        }
        other => panic!("expected ConfigSub::Set, got {other:?}"),
    }
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
    let cli2 = parse(&["opencoder", "server", "--port", "1", "--token", "abc"]);
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
            interrupt,
            prompt,
        }) => {
            assert_eq!(remote, "http://127.0.0.1:8080");
            assert_eq!(token.as_deref(), Some("TKN"));
            assert!(session.is_none());
            assert!(!continue_);
            assert_eq!(prompt, vec!["do".to_string(), "the thing".to_string()]);
            // --agent is now a global flag (not a client-local field)
            assert!(cli.agent.is_none());
            assert!(!interrupt);
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

#[test]
fn client_subcommand_parses_agent_model_interrupt() {
    use opencoder_cli::Command;
    // --agent and --model are both global flags; --interrupt is client-local.
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--agent",
        "build",
        "--model",
        "glm-5.2",
        "--interrupt",
        "--continue",
    ]);
    match cli.command {
        Some(Command::Client {
            interrupt,
            continue_,
            ..
        }) => {
            assert!(interrupt);
            assert!(continue_);
        }
        _ => panic!("expected Client"),
    }
    // the global --agent and --model are populated, not client-local fields
    assert_eq!(cli.agent.as_deref(), Some("build"));
    assert_eq!(cli.model.as_deref(), Some("glm-5.2"));
}

#[test]
fn ts_has_rs_alias() {
    // `rs` is an alias for the `ts` command, so `rs -l` works.
    let cli = parse(&["opencode", "rs", "-l"]);
    match cli.command {
        Some(Command::Ts { list, .. }) => assert!(list),
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn rs_alias_long_list_flag() {
    let cli = parse(&["opencode", "rs", "--list"]);
    match cli.command {
        Some(Command::Ts { list, .. }) => assert!(list),
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn rs_alias_resume_target() {
    let cli = parse(&["opencode", "rs", "-r", "01HZ"]);
    match cli.command {
        Some(Command::Ts { resume, .. }) => assert_eq!(resume.as_deref(), Some("01HZ")),
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn rs_alias_defaults() {
    let cli = parse(&["opencode", "rs"]);
    match cli.command {
        Some(Command::Ts { list, resume, new }) => {
            assert!(!list);
            assert!(resume.is_none());
            assert!(!new);
        }
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn run_subcommand_accepts_global_agent_flag() {
    // --agent is global, so it works on `run`, the bare path, and `client`.
    let cli = parse(&["opencoder", "run", "--agent", "plan", "design the api"]);
    match cli.command {
        Some(opencoder_cli::Command::Run { prompt }) => {
            assert_eq!(prompt, vec!["design the api".to_string()]);
        }
        _ => panic!("expected Run"),
    }
    assert_eq!(cli.agent.as_deref(), Some("plan"));
}

#[test]
fn bare_prompt_accepts_global_agent_flag() {
    // bare `opencode --agent explore "..."` also populates the global flag.
    let cli = parse(&["opencoder", "--agent", "explore", "explain it"]);
    assert_eq!(cli.agent.as_deref(), Some("explore"));
    assert!(!cli.prompt.is_empty());
}

#[test]
fn update_subcommand() {
    let cli = parse(&["opencoder", "update"]);
    assert!(matches!(cli.command, Some(Command::Update)));
}
