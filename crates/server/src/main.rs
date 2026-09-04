//! `opencoder-server` — the fleet control-plane binary.
//!
//! Web API + SPA + DAG dispatch/record ONLY: it never executes workflows and
//! never links the VM/runc chain (those live in `opencoder-agent`). Extracted
//! from the former `opencode daemon --server` arm when the project split
//! into three binaries (tui/cli, server, agent).

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "opencoder-server",
    version,
    about = "opencode fleet control plane: web API + SPA + DAG dispatch (no local execution)"
)]
struct Args {
    /// Bind host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Bind port; 0 lets the OS pick a free one.
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Serve the bundled web frontend.
    #[arg(long, default_value_t = true)]
    web: bool,
    /// Bearer token: --token, then OPENCODER_SERVER_TOKEN, else an
    /// auto-generated token printed to stderr for handing to clients.
    #[arg(long)]
    token: Option<String>,
    /// Directory the server operates on (config + data dir discovery).
    #[arg(long)]
    workdir: Option<PathBuf>,
    /// Verbose logging (repeatable).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

/// Token priority: flag, env, then an auto-generated ULID printed to stderr.
/// Same semantics as the old `cli::server::resolve_token` (the server side
/// MAY invent a token because clients learn it from the operator).
fn resolve_token(flag: Option<String>) -> String {
    if let Some(t) = flag {
        return t;
    }
    if let Ok(t) = std::env::var("OPENCODER_SERVER_TOKEN") {
        if !t.trim().is_empty() {
            return t;
        }
    }
    let t = ulid::Ulid::new().to_string();
    eprintln!("opencoder-server: generated bearer token: {t}");
    eprintln!("  pass it to agents via --token or OPENCODER_SERVER_TOKEN");
    t
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    opencoder_cli_compat::init_logging(args.verbose);
    let workdir = args
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let token = resolve_token(args.token);
    opencoder_web::serve(args.host, args.port, args.web, workdir, token).await
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
                "opencoder_web={level},opencoder_server={level}"
            ))
        });
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_token;

    /// The flag always wins and never consults the process env, so the
    /// assertion is deterministic regardless of OPENCODER_SERVER_TOKEN.
    #[test]
    fn resolve_token_param_wins() {
        assert_eq!(resolve_token(Some("explicit".into())), "explicit");
    }

    #[test]
    fn resolve_token_generated_is_ulid_shaped() {
        // No flag; isolate from a possible ambient env var by overriding it.
        std::env::set_var("OPENCODER_SERVER_TOKEN", "  ");
        let t = resolve_token(None);
        assert!(
            ulid::Ulid::from_string(&t).is_ok(),
            "generated token must be a ULID: {t}"
        );
    }
}
