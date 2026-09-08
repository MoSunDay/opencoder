//! `opencoder-server` — the fleet control-plane binary.
//!
//! Web console, global definitions, brain and node scheduling: no local execution.
//! It never executes workflows and
//! never links the VM/runc chain (those live in `opencoder-agent`). Extracted
//! from the former `opencoder daemon --server` arm when the project split
//! into three binaries (tui/cli, server, agent).

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "opencoder-server",
    version,
    long_version = opencoder_core::version::VERSION_LONG,
    about = "opencoder fleet control plane: web console, brain and node scheduling"
)]
struct Args {
    /// Print machine-readable version, commit and fleet protocol metadata.
    #[arg(long)]
    build_info: bool,
    /// Bind host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Bind port; 0 lets the OS pick a free one.
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Serve the bundled web frontend.
    #[arg(long, default_value_t = true)]
    web: bool,
    /// Bearer token. Mutually exclusive with --token-file.
    #[arg(long, conflicts_with = "token_file")]
    token: Option<String>,
    /// Read the Bearer token from a credential file.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,
    /// Directory the server operates on (config + data dir discovery).
    #[arg(long)]
    workdir: Option<PathBuf>,
    /// Control-plane persistent data directory. Defaults to the existing
    /// per-workdir `server-v2` location when omitted.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Verbose logging (repeatable).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn token_value(value: String, source: &str) -> Result<String> {
    let token = value.trim();
    anyhow::ensure!(!token.is_empty(), "{source} contains an empty bearer token");
    Ok(token.to_owned())
}

fn resolve_token(flag: Option<String>, file: Option<PathBuf>) -> Result<String> {
    anyhow::ensure!(
        flag.is_none() || file.is_none(),
        "--token and --token-file are mutually exclusive"
    );
    if let Some(value) = flag {
        return token_value(value, "--token");
    }
    if let Some(path) = file {
        let value = std::fs::read_to_string(&path)
            .with_context(|| format!("read token file {}", path.display()))?;
        return token_value(value, "token file");
    }
    match std::env::var("OPENCODER_SERVER_TOKEN") {
        Ok(value) => token_value(value, "OPENCODER_SERVER_TOKEN"),
        Err(_) => anyhow::bail!(
            "server token required: pass --token, --token-file, or set OPENCODER_SERVER_TOKEN"
        ),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.build_info {
        println!("{}", opencoder_core::version::build_info_json());
        return Ok(());
    }
    opencoder_cli_compat::init_logging(args.verbose);
    let workdir = args
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let token = resolve_token(args.token, args.token_file)?;
    opencoder_control::serve(
        args.host,
        args.port,
        args.web,
        workdir,
        args.data_dir,
        token,
    )
    .await
}

/// Tiny local logging bootstrap (the cli crate owns the shared one; the
/// server keeps its dependency surface minimal on purpose).
mod opencoder_cli_compat {
    pub fn init_logging(verbose: u8) {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            let level = match verbose {
                0 => "info",
                1 => "debug",
                _ => "trace",
            };
            tracing_subscriber::EnvFilter::new(format!(
                "opencoder_web={level},opencoder_server={level},opencoder_control={level}"
            ))
        });
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_token, Args};
    use clap::Parser;

    /// The flag always wins and never consults the process env, so the
    /// assertion is deterministic regardless of OPENCODER_SERVER_TOKEN.
    #[test]
    fn resolve_token_param_wins() {
        assert_eq!(
            resolve_token(Some("explicit".into()), None).unwrap(),
            "explicit"
        );
    }

    #[test]
    fn token_file_is_trimmed_and_empty_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "case-Sensitive\n").unwrap();
        assert_eq!(
            resolve_token(None, Some(path.clone())).unwrap(),
            "case-Sensitive"
        );
        std::fs::write(&path, " \n").unwrap();
        assert!(resolve_token(None, Some(path)).is_err());
        assert!(resolve_token(None, Some(dir.path().join("missing"))).is_err());
    }

    #[test]
    fn token_flags_are_mutually_exclusive() {
        assert!(Args::try_parse_from([
            "opencoder-server",
            "--token",
            "one",
            "--token-file",
            "/run/credentials/token"
        ])
        .is_err());
    }
}
