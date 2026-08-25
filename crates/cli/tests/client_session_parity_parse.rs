//! Parse assertions for the 2026-08-25 client session parity subcommands
//! (`session tasks` / `session clear <keep>`), split out of `cli_parse.rs`
//! to respect the file-size budget. Semantics covered by
//! `crates/web/tests/client_remote_ops.rs` (server roundtrip).

use clap::Parser;
use opencoder_cli::{Cli, ClientSessionSub, ClientSub, Command};

fn parse(args: &[&str]) -> Cli {
    Cli::parse_from(args)
}

#[test]
fn client_session_tasks_parses_positional_id() {
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://127.0.0.1:8080",
        "--token",
        "TKN",
        "session",
        "tasks",
        "sess-9",
    ]);
    match cli.command {
        Some(Command::Client {
            cmd: Some(ClientSub::Session { sub: ClientSessionSub::Tasks { id } }),
            ..
        }) => assert_eq!(id, "sess-9"),
        other => panic!("expected client session tasks, got {other:?}"),
    }
}

#[test]
fn client_session_clear_parses_positional_keep() {
    let cli = parse(&[
        "opencoder",
        "client",
        "--remote",
        "http://127.0.0.1:8080",
        "--token",
        "TKN",
        "session",
        "clear",
        "keep-1",
    ]);
    match cli.command {
        Some(Command::Client {
            cmd: Some(ClientSub::Session { sub: ClientSessionSub::Clear { keep } }),
            ..
        }) => assert_eq!(keep, "keep-1"),
        other => panic!("expected client session clear, got {other:?}"),
    }
}
