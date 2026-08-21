use clap::Parser;
use opencoder_cli::{
    Cli, ClientQuestionsSub, ClientSessionSub, ClientSub, Command, ConfigSub, SessionSub,
};

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
            ..
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

    // Bug 12: --session + --continue are mutually exclusive in Client
    let cli2 = Cli::try_parse_from([
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--session",
        "01ABC",
        "--continue",
        "hi",
    ]);
    assert!(
        cli2.is_err(),
        "--session and --continue must conflict in Client"
    );

    // Bug 12: --session + --continue are mutually exclusive in top-level Cli
    let cli3 = Cli::try_parse_from(["opencoder", "--session", "01ABC", "--continue"]);
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

// ── client: new flags (delivery/skill/fork/compact/handoff/autopilot/
// annotation/steer-task/workdir) ────────────────────────────────────────────

#[test]
fn client_new_flags_parse() {
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--delivery",
        "queue",
        "--skill",
        "a",
        "--skill",
        "b",
        "--fork",
        "--compact",
        "--autopilot",
        "ap",
        "--annotation",
        "text",
        "--steer-task",
        "t1",
        "hello",
    ]);
    match cli.command {
        Some(Command::Client {
            delivery,
            skills,
            fork,
            compact,
            autopilot,
            annotation,
            steer_task,
            prompt,
            ..
        }) => {
            assert_eq!(delivery, "queue");
            assert_eq!(skills, vec!["a".to_string(), "b".to_string()]);
            assert!(fork);
            assert!(compact);
            assert_eq!(autopilot.as_deref(), Some("ap"));
            assert_eq!(annotation.as_deref(), Some("text"));
            assert_eq!(steer_task.as_deref(), Some("t1"));
            assert_eq!(prompt, vec!["hello".to_string()]);
        }
        _ => panic!("expected Client"),
    }

    // --workdir is the GLOBAL flag (accepted in subcommand position) and acts
    // as the remote session filter for --continue / `client session list`.
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--workdir",
        "/tmp",
        "hello",
    ]);
    assert_eq!(cli.workdir.as_deref(), Some(std::path::Path::new("/tmp")));
    match cli.command {
        Some(Command::Client { prompt, .. }) => {
            assert_eq!(prompt, vec!["hello".to_string()]);
        }
        _ => panic!("expected Client"),
    }
}

#[test]
fn client_delivery_defaults_to_steer() {
    let cli = parse(&["opencoder", "client", "--remote", "http://x", "hi"]);
    match cli.command {
        Some(Command::Client { delivery, .. }) => assert_eq!(delivery, "steer"),
        _ => panic!("expected Client"),
    }
}

#[test]
fn client_handoff_parses_with_and_without_extra() {
    // bare --handoff -> Some("") (default_missing_value)
    let cli = parse(&["opencoder", "client", "--remote", "http://x", "--handoff"]);
    match cli.command {
        Some(Command::Client { handoff, .. }) => {
            assert_eq!(handoff.as_deref(), Some(""));
        }
        _ => panic!("expected Client"),
    }
    // --handoff with a positional extra
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--handoff",
        "extra text",
    ]);
    match cli.command {
        Some(Command::Client { handoff, .. }) => {
            assert_eq!(handoff.as_deref(), Some("extra text"));
        }
        _ => panic!("expected Client"),
    }
}

#[test]
fn client_unknown_delivery_and_autopilot_values_still_parse() {
    // Runtime-validated (server 400s on delivery; client_run rejects autopilot):
    // clap itself must accept arbitrary values so tests pin that contract.
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--delivery",
        "typo",
        "--autopilot",
        "bogus",
        "hi",
    ]);
    match cli.command {
        Some(Command::Client {
            delivery,
            autopilot,
            ..
        }) => {
            assert_eq!(delivery, "typo");
            assert_eq!(autopilot.as_deref(), Some("bogus"));
        }
        _ => panic!("expected Client"),
    }
}

// ── client: management subcommands ────────────────────────────────────────

#[test]
fn client_session_subcommands_parse() {
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "session",
        "list",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Session {
                    sub: ClientSessionSub::List,
                }),
            ..
        }) => {}
        _ => panic!("expected client session list"),
    }
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "session",
        "show",
        "id1",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Session {
                    sub: ClientSessionSub::Show { id },
                }),
            ..
        }) => assert_eq!(id, "id1"),
        _ => panic!("expected client session show"),
    }
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "session",
        "delete",
        "id1",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Session {
                    sub: ClientSessionSub::Delete { id },
                }),
            ..
        }) => assert_eq!(id, "id1"),
        _ => panic!("expected client session delete"),
    }
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "session",
        "fork",
        "id1",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Session {
                    sub: ClientSessionSub::Fork { id },
                }),
            ..
        }) => assert_eq!(id, "id1"),
        _ => panic!("expected client session fork"),
    }
}

#[test]
fn client_questions_subcommands_parse() {
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "questions",
        "list",
        "s1",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Questions {
                    sub: ClientQuestionsSub::List { session },
                }),
            ..
        }) => assert_eq!(session, "s1"),
        _ => panic!("expected client questions list"),
    }
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "questions",
        "answer",
        "s1",
        "c1",
        "yes",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Questions {
                    sub:
                        ClientQuestionsSub::Answer {
                            session,
                            call_id,
                            answer,
                        },
                }),
            ..
        }) => {
            assert_eq!(session, "s1");
            assert_eq!(call_id, "c1");
            assert_eq!(answer, "yes");
        }
        _ => panic!("expected client questions answer"),
    }
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "questions",
        "skip",
        "s1",
        "c1",
    ])
    .command
    {
        Some(Command::Client {
            cmd:
                Some(ClientSub::Questions {
                    sub: ClientQuestionsSub::Skip { session, call_id },
                }),
            ..
        }) => {
            assert_eq!(session, "s1");
            assert_eq!(call_id, "c1");
        }
        _ => panic!("expected client questions skip"),
    }
}

#[test]
fn client_subcommand_wins_over_prompt_shaped_text() {
    // `session`/`questions` as the FIRST prompt word is captured by the
    // subcommand (documented: use `--` to force the prompt path).
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "session",
        "list",
    ])
    .command
    {
        Some(Command::Client { cmd, .. }) => assert!(cmd.is_some()),
        _ => panic!("expected Client"),
    }
    // The `--` workaround keeps such words in the prompt.
    match parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://x",
        "--",
        "session",
        "list",
    ])
    .command
    {
        Some(Command::Client { cmd, prompt, .. }) => {
            assert!(cmd.is_none());
            assert_eq!(prompt, vec!["session".to_string(), "list".to_string()]);
        }
        _ => panic!("expected Client"),
    }
}
