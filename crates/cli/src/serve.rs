use std::path::PathBuf;

use anyhow::Result;

use crate::Cli;

/// Resolve the effective workdir for a subcommand: honor the global `--workdir`
/// flag, else fall back to the process cwd. The `serve` subcommand MUST go
/// through this so `opencoder --workdir X serve` actually serves X (previously
/// it silently used cwd, ignoring --workdir).
fn resolve_workdir(cli: &Cli) -> PathBuf {
    cli.workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub async fn serve_run(cli: &Cli, host: String, port: u16, web: bool) -> Result<()> {
    opencode_web::serve(host, port, web, resolve_workdir(cli)).await
}

pub async fn serve_launch(cli: &Cli) -> Result<()> {
    opencode_web::serve("127.0.0.1".to_string(), 0, true, resolve_workdir(cli)).await
}
