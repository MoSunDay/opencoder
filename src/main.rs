use anyhow::Result;
use clap::Parser;
use opencode_cli::{init_logging, Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match &cli.command {
        Some(Command::Run { prompt }) => {
            let parts = if prompt.is_empty() { cli.prompt.clone() } else { prompt.clone() };
            let p = join(parts);
            require(&p)?;
            opencode_cli::run::run_headless(&cli, p).await
        }
        Some(Command::Serve { host, port, web }) => {
            opencode_cli::serve::serve_run(&cli, host.clone(), *port, *web).await
        }
        Some(Command::Tui) => opencode_tui::run_tui(&opts_from_cli(&cli)).await,
        Some(Command::Config { sub }) => opencode_cli::session_cmd::config_dispatch(&cli, sub).await,
        Some(Command::Models) => opencode_cli::session_cmd::models_dispatch(&cli).await,
        Some(Command::Session { sub }) => opencode_cli::session_cmd::session_dispatch(sub, &cli).await,
        None => {
            if !cli.prompt.is_empty() {
                let p = join(cli.prompt.clone());
                require(&p)?;
                opencode_cli::run::run_headless(&cli, p).await
            } else {
                opencode_tui::run_tui(&opts_from_cli(&cli)).await
            }
        }
    }
}

fn opts_from_cli(cli: &Cli) -> opencode_tui::TuiOpts {
    opencode_tui::TuiOpts::new(cli.model.clone(), cli.agent.clone(), cli.workdir.clone())
}

fn join(parts: Vec<String>) -> String {
    parts.join(" ").trim().to_string()
}

fn require(p: &str) -> Result<()> {
    if p.is_empty() {
        return Err(anyhow::anyhow!(
            "no prompt provided. Usage: opencode \"your prompt\"  |  opencode run \"...\""
        ));
    }
    Ok(())
}
