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
fn session_and_continue_conflict_globally() {
    // Bug 12 contract, kept after the client subcommand removal: --session and
    // --continue remain mutually exclusive on the top-level Cli.
    let cli3 = Cli::try_parse_from(["opencode", "--session", "01ABC", "--continue"]);
    assert!(
        cli3.is_err(),
        "--session and --continue must conflict in top-level Cli"
    );
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
    let cli = parse(&["opencoder", "ts", "-l"]);
    match cli.command {
        Some(Command::Ts {
            list,
            resume,
            clean,
            delete,
        }) => {
            assert!(list);
            assert!(resume.is_none());
            assert!(!clean);
            assert!(delete.is_none());
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_parses_resume_target() {
    let cli = parse(&["opencoder", "ts", "-r", "01HZ"]);
    match cli.command {
        Some(Command::Ts {
            list,
            resume,
            clean,
            delete,
        }) => {
            assert!(!list);
            assert_eq!(resume.as_deref(), Some("01HZ"));
            assert!(!clean);
            assert!(delete.is_none());
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_defaults_to_no_flags() {
    // Bare `opencode ts` -> Ts with every flag at its default.
    let cli = parse(&["opencoder", "ts"]);
    match cli.command {
        Some(Command::Ts {
            list,
            resume,
            clean,
            delete,
        }) => {
            assert!(!list);
            assert!(resume.is_none());
            assert!(!clean);
            assert!(delete.is_none());
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_rejects_new_flag() {
    // `--new` was removed: creating is now the default behavior, so clap must
    // reject it as an unknown argument.
    use clap::error::ErrorKind;
    let res = opencoder_cli::Cli::try_parse_from(["opencoder", "ts", "--new"]);
    assert!(res.is_err(), "--new should be rejected");
    let kind = res.unwrap_err().kind();
    assert!(
        matches!(kind, ErrorKind::UnknownArgument),
        "--new must be an unknown argument, got {kind:?}"
    );
}

#[test]
fn ts_has_rs_alias() {
    // `rs` is an alias for the `ts` command, so `rs -l` works.
    let cli = parse(&["opencoder", "rs", "-l"]);
    match cli.command {
        Some(Command::Ts { list, .. }) => assert!(list),
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn rs_alias_long_list_flag() {
    let cli = parse(&["opencoder", "rs", "--list"]);
    match cli.command {
        Some(Command::Ts { list, .. }) => assert!(list),
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn rs_alias_resume_target() {
    let cli = parse(&["opencoder", "rs", "-r", "01HZ"]);
    match cli.command {
        Some(Command::Ts { resume, .. }) => assert_eq!(resume.as_deref(), Some("01HZ")),
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn rs_alias_defaults() {
    let cli = parse(&["opencoder", "rs"]);
    match cli.command {
        Some(Command::Ts {
            list,
            resume,
            clean,
            delete,
        }) => {
            assert!(!list);
            assert!(resume.is_none());
            assert!(!clean);
            assert!(delete.is_none());
        }
        _ => panic!("expected Ts via rs alias"),
    }
}

#[test]
fn run_subcommand_accepts_global_agent_flag() {
    // --agent is global, so it works on `run` and the bare prompt path.
    let cli = parse(&["opencoder", "run", "--agent", "sandbox", "design the api"]);
    match cli.command {
        Some(opencoder_cli::Command::Run { prompt }) => {
            assert_eq!(prompt, vec!["design the api".to_string()]);
        }
        _ => panic!("expected Run"),
    }
    assert_eq!(cli.agent.as_deref(), Some("sandbox"));
}

#[test]
fn bare_prompt_accepts_global_agent_flag() {
    // bare `opencode --agent explore "..."` also populates the global flag.
    let cli = parse(&["opencoder", "--agent", "explore", "explain it"]);
    assert_eq!(cli.agent.as_deref(), Some("explore"));
    assert!(!cli.prompt.is_empty());
}

#[test]
fn agent_flag_rejects_removed_plan_agent_with_rename_hint() {
    // The removed plan/act dual mode: `--agent plan` must fail at parse time
    // (not late, after resume bookkeeping) and point at the `sandbox` rename.
    let err = Cli::try_parse_from(["opencoder", "--agent", "plan", "hi"]).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid value 'plan'"),
        "expected clap invalid-value error, got: {msg}"
    );
    assert!(
        msg.contains("sandbox"),
        "error must mention the sandbox replacement, got: {msg}"
    );
}

#[test]
fn agent_flag_rejects_unknown_names_at_parse_time() {
    // Any non-builtin fails the same way --agent validation is not silently
    // downgraded to the default agent.
    let err =
        Cli::try_parse_from(["opencoder", "--agent", "nonexistent-agent", "hi"]).unwrap_err();
    assert!(
        err.to_string().contains("nonexistent-agent"),
        "error must name the offending value, got: {err}"
    );
}

#[test]
fn agent_flag_accepts_documented_primaries() {
    // Every primary documented on the flag parses; other resolvable builtins
    // (command/workflow) stay accepted so the parser never over-restricts
    // beyond `resolve_agent`.
    for name in ["act", "sandbox", "explore", "build", "command", "workflow"] {
        let cli = parse(&["opencoder", "--agent", name, "hi"]);
        assert_eq!(cli.agent.as_deref(), Some(name), "agent {name} must parse");
    }
}

#[test]
fn update_subcommand() {
    let cli = parse(&["opencoder", "update"]);
    assert!(matches!(cli.command, Some(Command::Update)));
}

#[test]
fn install_tools_subcommand() {
    let cli = parse(&["opencoder", "install-tools"]);
    assert!(matches!(cli.command, Some(Command::InstallTools)));
}

#[test]
fn ts_subcommand_parses_clean_flag() {
    let cli = parse(&["opencoder", "ts", "-c"]);
    match cli.command {
        Some(Command::Ts {
            list,
            resume,
            clean,
            delete,
        }) => {
            assert!(!list);
            assert!(resume.is_none());
            assert!(clean);
            assert!(delete.is_none());
        }
        _ => panic!("expected Ts"),
    }
}

#[test]
fn ts_subcommand_parses_delete_target_and_rejects_mixed_actions() {
    let cli = parse(&["opencoder", "ts", "-d", "01HZ"]);
    match cli.command {
        Some(Command::Ts { delete, .. }) => assert_eq!(delete.as_deref(), Some("01HZ")),
        _ => panic!("expected Ts delete"),
    }
    assert!(opencoder_cli::Cli::try_parse_from(["opencoder", "ts", "-d", "01HZ", "-c"]).is_err());
}
