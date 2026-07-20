use anyhow::Result;
use clap::Parser;
use opencoder_cli::{init_logging, Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // The TUI runs in the alternate screen + raw mode, so any log line written
    // to stdout/stderr overlays the interface as garbage. Route TUI logs to a
    // file instead; headless commands keep logging on stdout.
    let is_tui = matches!(cli.command, Some(Command::Tui))
        || (cli.command.is_none() && cli.prompt.is_empty());
    let log_sink = if is_tui {
        opencoder_cli::tui_log_path()
    } else {
        None
    };
    init_logging(cli.verbose, log_sink.as_deref());

    // Seed the built-in skill packs (do-and-done, repo-local-memory, review,
    // submit) into ~/.opencoder/skills on first run. Idempotent: a no-op once
    // the `review` skill directory exists.
    opencoder_core::seed_builtin_skills();

    match &cli.command {
        Some(Command::Run { prompt }) => {
            let parts = if prompt.is_empty() {
                cli.prompt.clone()
            } else {
                prompt.clone()
            };
            let p = join(parts);
            require(&p)?;
            opencoder_cli::run::run_headless(&cli, p).await
        }
        Some(Command::Server { host, port, web, token }) => {
            opencoder_cli::server::server_run(&cli, host.clone(), *port, *web, token.clone()).await
        }
        Some(Command::Client { remote, token, session, continue_, prompt }) => {
            let parts = if prompt.is_empty() {
                cli.prompt.clone()
            } else {
                prompt.clone()
            };
            let p = join(parts);
            require(&p)?;
            opencoder_cli::client::client_run(
                remote.clone(),
                token.clone(),
                session.clone(),
                *continue_,
                p,
            )
            .await
        }
        Some(Command::Tui) => opencoder_tui::run_tui(&opts_from_cli(&cli)).await,
        Some(Command::Config { sub }) => {
            opencoder_cli::session_cmd::config_dispatch(&cli, sub).await
        }
        Some(Command::Models) => opencoder_cli::session_cmd::models_dispatch(&cli).await,
        Some(Command::Session { sub }) => {
            opencoder_cli::session_cmd::session_dispatch(sub, &cli).await
        }
        None => {
            if !cli.prompt.is_empty() {
                let p = join(cli.prompt.clone());
                require(&p)?;
                opencoder_cli::run::run_headless(&cli, p).await
            } else {
                opencoder_tui::run_tui(&opts_from_cli(&cli)).await
            }
        }
    }
}

fn opts_from_cli(cli: &Cli) -> opencoder_tui::TuiOpts {
    opencoder_tui::TuiOpts::new(cli.model.clone(), cli.agent.clone(), cli.workdir.clone())
        .with_session(cli.session.clone())
}

fn join(parts: Vec<String>) -> String {
    parts.join(" ").trim().to_string()
}

fn require(p: &str) -> Result<()> {
    if p.is_empty() {
        return Err(anyhow::anyhow!(
            "no prompt provided. Usage: opencoder \"your prompt\"  |  opencoder run \"...\""
        ));
    }
    Ok(())
}
