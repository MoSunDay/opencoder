use clap::Parser;
use opencoder_cli::{Cli, Command};

fn parse(args: &[&str]) -> Cli {
    Cli::parse_from(args)
}

#[test]
fn daemon_server_mode_parses_host_port_and_token() {
    let cli = parse(&[
        "opencoder",
        "daemon",
        "--server",
        "--port",
        "9090",
        "--host",
        "127.0.0.1",
    ]);
    match cli.command {
        Some(Command::Daemon {
            server,
            client,
            opts,
        }) => {
            assert!(server);
            assert!(!client);
            assert_eq!(opts.port, 9090);
            assert_eq!(opts.host, "127.0.0.1");
        }
        _ => panic!("expected Daemon --server"),
    }

    // The server branch still accepts --token (resolved server-side: flag,
    // then OPENCODER_SERVER_TOKEN, then auto-generate).
    let cli2 = parse(&[
        "opencoder",
        "daemon",
        "--server",
        "--port",
        "1",
        "--token",
        "abc",
    ]);
    match cli2.command {
        Some(Command::Daemon { opts, .. }) => {
            assert_eq!(opts.token.as_deref(), Some("abc"));
        }
        _ => panic!("expected Daemon --server"),
    }
}

#[test]
fn daemon_bare_invocation_is_rejected() {
    // Exactly one of --server / --client is required: clap must reject a bare
    // `daemon` instead of letting it fall through to dispatch.
    assert!(
        Cli::try_parse_from(["opencoder", "daemon"]).is_err(),
        "bare `daemon` must fail to parse"
    );
}

#[test]
fn daemon_server_and_client_are_mutually_exclusive() {
    let res = Cli::try_parse_from([
        "opencoder",
        "daemon",
        "--server",
        "--client",
        "--remote",
        "u",
    ]);
    assert!(res.is_err(), "--server + --client must fail to parse");
}

#[test]
fn daemon_client_mode_parses_name_token_and_remote() {
    let cli = parse(&[
        "opencoder",
        "--workdir",
        "/tmp/nodework",
        "daemon",
        "--client",
        "--remote",
        "http://127.0.0.1:8080",
        "--name",
        "gpu-1",
        "--token",
        "tok-1",
    ]);
    match cli.command {
        Some(Command::Daemon {
            server,
            client,
            opts,
        }) => {
            assert!(!server);
            assert!(client);
            assert_eq!(opts.remote.as_deref(), Some("http://127.0.0.1:8080"));
            assert_eq!(opts.name.as_deref(), Some("gpu-1"));
            assert_eq!(opts.token.as_deref(), Some("tok-1"));
        }
        other => panic!("expected Daemon --client, got {other:?}"),
    }
    assert_eq!(
        cli.workdir.as_deref(),
        Some(std::path::Path::new("/tmp/nodework"))
    );
}

#[test]
fn daemon_client_without_remote_still_parses_remote_is_enforced_at_dispatch() {
    // Parse stays permissive; the --remote requirement is enforced by
    // daemon_mode() at dispatch with a plain usage error (see daemon.rs).
    let cli = parse(&["opencoder", "daemon", "--client"]);
    match cli.command {
        Some(Command::Daemon { client, opts, .. }) => {
            assert!(client);
            assert!(opts.remote.is_none());
        }
        other => panic!("expected Daemon --client, got {other:?}"),
    }
}
