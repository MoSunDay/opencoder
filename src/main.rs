use anyhow::Result;
use clap::Parser;
use opencoder_cli::{init_logging, Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // The TUI runs in the alternate screen + raw mode, so any log line written
    // to stdout/stderr overlays the interface as garbage. Route TUI logs to a
    // file instead; headless commands keep logging on stdout.
    let is_tui = matches!(cli.command, Some(Command::Tui) | Some(Command::Ts { .. }))
        || (cli.command.is_none() && cli.prompt.is_empty());
    let log_sink = if is_tui {
        opencoder_cli::tui_log_path()
    } else {
        None
    };
    init_logging(cli.verbose, log_sink.as_deref());

    // Seed the built-in skill packs into ~/.opencoder/skills. Incremental:
    // missing skills are written, existing files are never clobbered, so a
    // binary upgrade lands new built-in skills on the next startup.
    opencoder_core::seed_builtin_skills();
    opencoder_core::seed_dep_gated_skills();
    opencoder_core::write_install_script();

    let result = match &cli.command {
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
        Some(Command::Server {
            host,
            port,
            web,
            token,
        }) => {
            opencoder_cli::server::server_run(&cli, host.clone(), *port, *web, token.clone()).await
        }
        Some(Command::Client {
            remote,
            token,
            session,
            continue_,
            interrupt,
            prompt,
        }) => {
            let parts = if prompt.is_empty() {
                cli.prompt.clone()
            } else {
                prompt.clone()
            };
            let p = join(parts);
            if !*interrupt {
                require(&p)?;
            }
            // The Client subcommand re-declares its own --session/--continue,
            // which shadow the global flags. Fall back to the globals so
            // `opencode --continue client -r http://...` works as expected
            // instead of silently creating a fresh remote session.
            let (session, continue_) = opencoder_cli::client::resolve_client_session_flags(
                session.clone(),
                *continue_,
                cli.session.clone(),
                cli.continue_,
            );
            opencoder_cli::client::client_run(
                remote.clone(),
                token.clone(),
                session,
                continue_,
                cli.agent.clone(),
                cli.model.clone(),
                *interrupt,
                cli.image.clone(),
                p,
            )
            .await
        }
        Some(Command::Tui) => opencoder_tui::run_tui(&opts_from_cli(&cli)).await,
        Some(Command::Ts {
            list,
            resume,
            clean,
            delete,
        }) => {
            opencoder_cli::ts::ts_dispatch(
                &cli,
                *list,
                resume.as_deref(),
                *clean,
                delete.as_deref(),
            )
            .await
        }
        Some(Command::Config { sub }) => {
            opencoder_cli::session_cmd::config_dispatch(&cli, sub).await
        }
        Some(Command::Models) => opencoder_cli::session_cmd::models_dispatch(&cli).await,
        Some(Command::Session { sub }) => {
            opencoder_cli::session_cmd::session_dispatch(sub, &cli).await
        }
        Some(Command::Update) => opencoder_cli::update::update_run(&cli).await,
        None => {
            if !cli.prompt.is_empty() {
                let p = join(cli.prompt.clone());
                require(&p)?;
                opencoder_cli::run::run_headless(&cli, p).await
            } else if maybe_wrap_tui_in_tmux(&cli).await? {
                return Ok(());
            } else {
                opencoder_tui::run_tui(&opts_from_cli(&cli)).await
            }
        }
    };
    if is_tui {
        opencoder_cli::exit_tips::print_exit_tips();
    }
    // Kill any backgrounded bash commands (timeout handoff) and remove their
    // temp output files before the process exits.
    opencoder_session::tools::bg::cleanup_all();
    result
}

fn opts_from_cli(cli: &Cli) -> opencoder_tui::TuiOpts {
    opencoder_tui::TuiOpts::new(cli.workdir.clone())
        .with_session(cli.session.clone())
        .with_model(cli.model.clone())
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

/// When `enable_tmux_session` is set in config and tmux is available and we're
/// not already inside tmux, wrap the TUI in a tmux session. Returns `true` if
/// the TUI was launched inside tmux, `false` to fall through to the plain TUI.
async fn maybe_wrap_tui_in_tmux(cli: &Cli) -> Result<bool> {
    if opencoder_cli::ts::inside_tmux() || !opencoder_cli::ts::tmux_available() {
        return Ok(false);
    }
    let workdir = match &cli.workdir {
        Some(w) => w.clone(),
        None => std::env::current_dir()?,
    };
    let config = opencoder_core::Config::load(&workdir)?;
    if config.enable_tmux_session.unwrap_or(false) {
        opencoder_cli::ts::ts_dispatch(cli, false, None, false, None).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}
