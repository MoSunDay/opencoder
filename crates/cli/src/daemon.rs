//! `opencoder daemon` support: role validation + the migration hint.
//!
//! The fleet roles moved into dedicated binaries (three-binary split):
//! `opencoder-server` (web API + SPA + DAG dispatch) and `opencoder-agent`
//! (prompt tasks + node-side DAG execution). The `daemon` subcommand keeps
//! parsing its old flags so existing scripts get a clean, greppable pointer
//! instead of a parse error, then prints the migration hint and exits 0.

use crate::DaemonOpts;

/// Which fleet role `opencoder daemon` was asked to run (now migrated).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum DaemonAction {
    /// Run the web server — now `opencoder-server`.
    Server,
    /// Register this machine as an execution node — now `opencoder-agent`.
    Client,
}

/// Pure mode validation for `opencoder daemon`. Clap already enforces
/// exactly-one-of (--server / --client) at parse time; this mirrors the rule
/// so the dispatch arm is total and the contract is testable without a
/// parser. `--remote` is only enforced here (not at parse time) so the usage
/// error is a plain, greppable message.
pub fn daemon_mode(
    server: bool,
    client: bool,
    remote: Option<&str>,
) -> std::result::Result<DaemonAction, String> {
    match (server, client) {
        (true, false) => Ok(DaemonAction::Server),
        (false, true) => match remote {
            Some(_) => Ok(DaemonAction::Client),
            None => Err("daemon --client requires --remote <URL> (server base URL)".to_string()),
        },
        (true, true) => Err(
            "--server and --client are mutually exclusive: pick exactly one daemon role"
                .to_string(),
        ),
        (false, false) => {
            Err("daemon needs exactly one role: pass --server or --client".to_string())
        }
    }
}

/// The operator-facing migration pointer for the migrated daemon roles.
/// Echoes the parsed flags into the suggested command line so the hint is
/// copy-pasteable.
pub fn migration_hint(action: DaemonAction, opts: &DaemonOpts) -> String {
    match action {
        DaemonAction::Server => {
            let mut cmd = format!("opencoder-server --host {} --port {}", opts.host, opts.port);
            if !opts.web {
                cmd.push_str(" --web=false");
            }
            if let Some(t) = &opts.token {
                cmd.push_str(&format!(" --token {t}"));
            }
            format!(
                "daemon --server has moved to the dedicated server binary.\n  run: {cmd}\n  (opencode no longer embeds the web API; see `opencoder-server --help`)"
            )
        }
        DaemonAction::Client => {
            let remote = opts.remote.clone().unwrap_or_default();
            let mut cmd = format!("opencoder-agent --remote {remote}");
            if let Some(n) = &opts.name {
                cmd.push_str(&format!(" --name {n}"));
            }
            if let Some(t) = &opts.token {
                cmd.push_str(&format!(" --token {t}"));
            }
            format!(
                "daemon --client has moved to the dedicated agent binary.\n  run: {cmd}\n  (prompt tasks and DAG workflow runs now execute on `opencoder-agent`; see `opencoder-agent --help`)"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(remote: Option<&str>) -> DaemonOpts {
        DaemonOpts {
            host: "127.0.0.1".into(),
            port: 8080,
            web: true,
            token: None,
            remote: remote.map(str::to_string),
            name: None,
        }
    }

    // -- daemon_mode --------------------------------------------------------

    #[test]
    fn server_only_selects_the_server_role() {
        assert_eq!(
            daemon_mode(true, false, None).unwrap(),
            DaemonAction::Server
        );
    }

    #[test]
    fn client_with_remote_selects_the_client_role() {
        assert_eq!(
            daemon_mode(false, true, Some("http://127.0.0.1:8080")).unwrap(),
            DaemonAction::Client
        );
    }

    #[test]
    fn client_without_remote_errs_mentioning_the_remote_flag() {
        let err = daemon_mode(false, true, None).unwrap_err();
        assert!(
            err.contains("--remote"),
            "error must mention --remote: {err}"
        );
    }

    #[test]
    fn both_roles_rejected() {
        assert!(daemon_mode(true, true, Some("u")).is_err());
    }

    #[test]
    fn no_role_rejected() {
        assert!(daemon_mode(false, false, None).is_err());
    }

    // -- migration_hint -----------------------------------------------------

    #[test]
    fn server_hint_names_the_new_binary_and_flags() {
        let mut o = opts(None);
        o.host = "0.0.0.0".into();
        o.port = 9090;
        let hint = migration_hint(DaemonAction::Server, &o);
        assert!(
            hint.contains("opencoder-server --host 0.0.0.0 --port 9090"),
            "{hint}"
        );
    }

    #[test]
    fn server_hint_echoes_token_and_web_off() {
        let mut o = opts(None);
        o.token = Some("TKN".into());
        o.web = false;
        let hint = migration_hint(DaemonAction::Server, &o);
        assert!(hint.contains("--token TKN"), "{hint}");
        assert!(hint.contains("--web=false"), "{hint}");
    }

    #[test]
    fn client_hint_carries_remote_name_token() {
        let mut o = opts(Some("http://s:8080"));
        o.name = Some("gpu-1".into());
        o.token = Some("TKN".into());
        let hint = migration_hint(DaemonAction::Client, &o);
        assert!(
            hint.contains("opencoder-agent --remote http://s:8080 --name gpu-1 --token TKN"),
            "{hint}"
        );
    }
}
