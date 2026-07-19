pub mod client;
pub mod run;
pub mod server;
pub mod session_cmd;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "opencoder",
    version,
    about = "High-performance minimal coding agent (Rust)"
)]
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
    #[arg(short, long, global = true)]
    pub session: Option<String>,
    /// Resume the most recent session for this workdir.
    #[arg(long, global = true, default_value_t = false)]
    pub continue_: bool,
    /// Fork (copy) the resumed session before continuing, leaving the original untouched.
    #[arg(long, global = true, default_value_t = false)]
    pub fork: bool,
    #[arg(long, global = true, default_value_t = false)]
    pub verbose: bool,
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
    /// Start the server: centralized storage + LLM gateway (HTTP/JSON + SSE),
    /// protected by a bearer token. (`serve` is accepted as an alias.)
    #[command(alias = "serve")]
    Server {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long, default_value_t = true)]
        web: bool,
        /// Bearer token for API auth. Defaults to OPENCODER_SERVER_TOKEN, then
        /// an auto-generated token printed to stderr.
        #[arg(long)]
        token: Option<String>,
    },
    /// Thin remote client: submit a prompt to a server and stream the result.
    /// Stores nothing locally and calls no LLM.
    Client {
        /// Server base URL (e.g. http://127.0.0.1:8080).
        #[arg(long)]
        remote: String,
        /// Bearer token. Defaults to OPENCODER_SERVER_TOKEN.
        #[arg(long)]
        token: Option<String>,
        /// Resume a specific remote session by id.
        #[arg(short, long)]
        session: Option<String>,
        /// Resume the most recent remote session.
        #[arg(long, default_value_t = false)]
        continue_: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
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
    Show {
        id: String,
        /// Emit full session state (meta + all message blocks + subagent
        /// tasks) as machine-readable JSON. Enables deep e2e assertions
        /// without coupling to storage internals.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Delete a session.
    Delete { id: String },
    /// Export a session (with subagent tree) to an opencoder binary file.
    Export {
        id: String,
        /// Output path. Defaults to `<id>.opencoder`.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Import a session from an opencoder binary file.
    Import {
        /// Path to the `.opencoder` bundle file.
        input: PathBuf,
    },
}

/// Path used to sink TUI logs so they never corrupt the alternate screen.
/// `<data_local_dir>/opencoder/tui.log`. Returns `None` if the data dir is
/// unavailable; the caller treats `None` as "log to stdout".
pub fn tui_log_path() -> Option<PathBuf> {
    let mut p = dirs::data_local_dir()?;
    p.push("opencoder");
    p.push("tui.log");
    Some(p)
}

/// Initialise the global tracing subscriber.
/// `file_sink`, when `Some`, directs log output to that file (truncated on
/// start). This is required for the TUI: the alternate screen + raw mode mean
/// any log written to stdout/stderr overlays the interface as garbage text
/// (e.g. the "WARN stream finished early" line). Headless commands pass `None`
/// to keep logging on stdout. Opening the file is best-effort — on failure we
/// fall back to stdout so logging never breaks the app.
pub fn init_logging(verbose: bool, file_sink: Option<&Path>) {
    let default_filter = if verbose {
        "debug"
    } else {
        "opencoder=info,warn"
    };
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));

    let file = file_sink.and_then(|p| std::fs::File::create(p).ok());
    match file {
        Some(f) => {
            let _ = tracing_subscriber::fmt()
                .with_writer(std::sync::Mutex::new(f))
                .with_env_filter(env_filter)
                .with_target(false)
                .try_init();
        }
        None => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(false)
                .try_init();
        }
    }
}
