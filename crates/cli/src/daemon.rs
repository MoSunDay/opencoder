//! `opencode daemon` support: mode validation, client-mode token resolution,
//! and node-name derivation. Pure decisions live here so the dispatch arm in
//! the binary stays a thin match and every rule is unit-testable without a
//! server, a parser, or a network.

use anyhow::{anyhow, Result};

/// Which fleet role `opencode daemon` should run.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum DaemonAction {
    /// Run the web server (registry + fleet dispatch + local engine).
    Server,
    /// Register this machine as an execution node.
    Client,
}

/// Pure mode validation for `opencode daemon`. Clap already enforces
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

/// Resolve the daemon CLIENT (node) bearer token: `--token` flag, then the
/// `OPENCODER_SERVER_TOKEN` environment variable. Unlike the server side, a
/// node NEVER auto-generates a token (an invented token could never
/// authenticate against the remote server).
pub fn resolve_client_token(flag: Option<String>) -> Result<String> {
    resolve_client_token_from(
        flag,
        std::env::var("OPENCODER_SERVER_TOKEN").ok().as_deref(),
    )
}

/// Pure core of [`resolve_client_token`] with the env value passed in, so the
/// flag / env / missing paths are all deterministic and unit-testable.
pub fn resolve_client_token_from(flag: Option<String>, env: Option<&str>) -> Result<String> {
    if let Some(t) = flag {
        return Ok(t);
    }
    env.filter(|t| !t.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("no token: pass --token <T> or set OPENCODER_SERVER_TOKEN"))
}

/// Default node `--name`: lowercase machine hostname trimmed to DNS-label
/// charset, disambiguated with a short process-local suffix so two nodes on
/// one host (or a container fleet sharing a hostname) stay distinct.
pub fn default_node_name() -> String {
    let raw = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "opencoder-node".into());
    let mut slug: String = raw
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if slug.is_empty() {
        slug = "opencoder-node".into();
    }
    let short = ulid::Ulid::new().to_string().to_lowercase();
    let tail: String = short.chars().rev().take(6).collect();
    format!("{slug}-{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn both_roles_err_even_though_clap_already_rejects_the_pair() {
        let err = daemon_mode(true, true, Some("http://x")).unwrap_err();
        assert!(
            err.contains("--server") && err.contains("--client"),
            "error must name both flags: {err}"
        );
    }

    #[test]
    fn neither_role_errs_with_usage_hint() {
        // Unreachable through the parser (clap enforces exactly-one) but the
        // function must stay total.
        let err = daemon_mode(false, false, None).unwrap_err();
        assert!(
            err.contains("--server") && err.contains("--client"),
            "error must advertise both roles: {err}"
        );
    }

    // -- resolve_client_token_from ------------------------------------------

    #[test]
    fn client_token_flag_wins_over_env() {
        let t = resolve_client_token_from(Some("explicit".into()), Some("from-env")).unwrap();
        assert_eq!(t, "explicit");
    }

    #[test]
    fn client_token_env_used_when_flag_absent() {
        let t = resolve_client_token_from(None, Some("from-env")).unwrap();
        assert_eq!(t, "from-env");
    }

    #[test]
    fn client_token_blank_env_is_treated_as_missing() {
        for blank in [None, Some(""), Some("   ")] {
            let err = resolve_client_token_from(None, blank)
                .expect_err("blank env must not authenticate");
            assert!(
                err.to_string().contains("OPENCODER_SERVER_TOKEN"),
                "error must mention OPENCODER_SERVER_TOKEN: {err}"
            );
        }
    }

    #[test]
    fn client_token_never_auto_generates() {
        // Distinguishing property vs the server resolver: the missing path is
        // an error, never a fresh ULID.
        assert!(resolve_client_token_from(None, None).is_err());
    }

    // -- default_node_name --------------------------------------------------

    #[test]
    fn default_node_name_is_nonempty_dns_label_with_unique_suffix() {
        let a = default_node_name();
        let b = default_node_name();
        assert!(!a.is_empty() && !b.is_empty());
        assert_ne!(a, b, "process-local suffix must disambiguate calls");
        for name in [a, b] {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'),
                "name must be lowercase DNS-label charset: {name}"
            );
        }
    }
}
