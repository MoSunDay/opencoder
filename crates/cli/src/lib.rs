pub mod run;
pub mod serve;
pub mod session_cmd;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "opencoder", version, about = "High-performance minimal coding agent (Rust)")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(short, long, global = true)]
    pub model: Option<String>,
    #[arg(long, global = true)]
    pub small_model: Option<String>,
    #[arg(long, global = true)]
    pub agent: Option<String>,
    #[arg(long, global = true)]
    pub workdir: Option<PathBuf>,
    /// Resume a specific session by id.
    #[arg(long, global = true)]
    pub session: Option<String>,
    /// Resume the most recent session for this workdir.
    #[arg(long, global = true, default_value_t = false)]
    pub continue_: bool,
    /// Fork (copy) the resumed session before continuing, leaving the original untouched.
    #[arg(long, global = true, default_value_t = false)]
    pub fork: bool,
    #[arg(long, global = true, default_value_t = false)]
    pub verbose: bool,
    #[arg(long, global = true, default_value_t = false)]
    pub serve: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub prompt: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Headless one-shot: run a prompt and stream output to stdout.
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },
    /// Start the interactive TUI.
    Tui,
    /// Start the HTTP/JSON API server with the web session manager.
    Serve {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long, default_value_t = true)]
        web: bool,
    },
    /// Print the resolved configuration (defaults < env < project file merged).
    Config {
        #[command(subcommand)]
        sub: Option<ConfigSub>,
    },
    /// List known models from the resolved config.
    Models,
    /// Session management (list / show / delete). Uses the local store.
    Session {
        #[command(subcommand)]
        sub: SessionSub,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigSub {
    /// Show the merged config as JSON.
    Show,
}

#[derive(Subcommand, Debug)]
pub enum SessionSub {
    /// List sessions for the current workdir.
    List,
    /// Show a session's messages.
    Show { id: String },
    /// Delete a session.
    Delete { id: String },
    /// Export a session (with subagent tree) to an opencode binary file.
    Export {
        id: String,
        /// Output path. Defaults to `<id>.opencode`.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Import a session from an opencode binary file.
    Import {
        /// Path to the `.opencode` bundle file.
        input: PathBuf,
    },
}

pub fn init_logging(verbose: bool) {
    let filter = if verbose { "debug" } else { "opencode=info,warn" };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_target(false)
        .try_init();
}
