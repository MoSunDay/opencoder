use clap::Parser;
use opencoder_cli::Cli;
use opencoder_core::harness::Harness;

#[test]
fn wrap_accepts_verbatim_requirement_and_repeated_environment() {
    let cli = Cli::try_parse_from([
        "opencoder",
        "--wrap",
        "codex",
        "--cmd",
        "中文 ' $(literal)\nnext",
        "--envs",
        "A= x=y ",
        "--envs",
        "B=",
    ])
    .unwrap();
    assert_eq!(cli.wrap, Some(Harness::Codex));
    assert_eq!(cli.cmd.as_deref(), Some("中文 ' $(literal)\nnext"));
    assert_eq!(
        cli.envs,
        vec![("A".into(), " x=y ".into()), ("B".into(), "".into())]
    );
}
#[test]
fn wrap_rejects_invalid_arguments_and_conflicting_prompts() {
    for args in [
        vec!["opencoder", "--wrap", "unknown"],
        vec!["opencoder", "--envs", "KEY"],
        vec!["opencoder", "--envs", "=value"],
        vec!["opencoder", "--cmd", "one", "two"],
        vec!["opencoder", "run", "--cmd", "one", "two"],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn wrap_globals_preserve_all_subcommand_definitions() {
    use clap::CommandFactory;
    Cli::command().debug_assert();
    let cli = Cli::try_parse_from([
        "opencoder",
        "run",
        "--wrap",
        "codex",
        "--cmd",
        "requirement",
    ])
    .unwrap();
    assert_eq!(cli.cmd.as_deref(), Some("requirement"));
}
