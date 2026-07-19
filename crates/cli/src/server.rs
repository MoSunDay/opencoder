use std::path::PathBuf;

use anyhow::Result;

use crate::Cli;

/// Resolve the effective workdir for a subcommand: honor the global `--workdir`
/// flag, else fall back to the process cwd. The server subcommand MUST go
/// through this so `opencoder --workdir X server` actually serves X (previously
/// it silently used cwd, ignoring --workdir).
fn resolve_workdir(cli: &Cli) -> PathBuf {
    cli.workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Resolve the bearer token by priority: `--token` flag, then the
/// `OPENCODER_SERVER_TOKEN` environment variable, then an auto-generated ULID
/// (printed to stderr so the operator can hand it to clients).
pub fn resolve_token(token: Option<String>) -> String {
    if let Some(t) = token {
        return t;
    }
    if let Ok(t) = std::env::var("OPENCODER_SERVER_TOKEN") {
        if !t.trim().is_empty() {
            return t;
        }
    }
    let t = ulid::Ulid::new().to_string();
    eprintln!("opencoder server: generated bearer token: {t}");
    eprintln!("  pass it to clients via --token or OPENCODER_SERVER_TOKEN");
    t
}

pub async fn server_run(
    cli: &Cli,
    host: String,
    port: u16,
    web: bool,
    token: Option<String>,
) -> Result<()> {
    let token = resolve_token(token);
    opencoder_web::serve(host, port, web, resolve_workdir(cli), token).await
}

#[cfg(test)]
mod tests {
    use super::resolve_token;

    /// `--token` flag always wins (and never depends on the process env, so the
    /// assertion is deterministic regardless of OPENCODER_SERVER_TOKEN).
    #[test]
    fn resolve_token_param_wins() {
        assert_eq!(resolve_token(Some("explicit".into())), "explicit");
    }
}
