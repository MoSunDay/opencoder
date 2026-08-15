pub mod client;
pub mod display;
pub mod exit_tips;
pub mod install_tools;
pub mod run;
mod run_image;
pub mod server;
pub mod session_cmd;
pub mod todos_cmd;
pub mod ts;
pub mod update;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "opencoder",
    version,
    long_version = opencoder_core::version::VERSION_LONG,
    about = "High-performance minimal coding agent (Rust)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(long, global = true)]
    pub workdir: Option<PathBuf>,
    /// Override the agent system prompt with the contents of this file.
    /// A standard bash/subagent usage preamble is appended automatically.
    #[arg(long, global = true)]
    pub prompt_file: Option<PathBuf>,
    /// Resume a specific session by id.
    #[arg(short, long, global = true, conflicts_with = "continue_")]
    pub session: Option<String>,
    /// Override the model for this run, as "{provider}/{model_id}"
    /// (e.g. "anthropic/claude-3"). Bound to the session: new sessions
    /// persist it; resuming with --model re-applies it (explicit choice
    /// wins over the stored model) and re-persists so later resumes honor it.
    #[arg(long, global = true, value_name = "MODEL")]
    pub model: Option<String>,
    /// Override the agent for this run, as a builtin name (act/plan/explore/build).
    /// New sessions use it as the primary agent and persist the choice; resuming
    /// with --agent re-applies it (explicit choice wins over the stored agent)
    /// and re-persists so later resumes honor it.
    #[arg(long, global = true, value_name = "AGENT")]
    pub agent: Option<String>,
    /// Resume the most recent session for this workdir.
    #[arg(
        long,
        global = true,
        default_value_t = false,
        conflicts_with = "session"
    )]
    pub continue_: bool,
    /// Fork (copy) the resumed session before continuing, leaving the original untouched.
    #[arg(long, global = true, default_value_t = false)]
    pub fork: bool,
    #[arg(long, global = true, default_value_t = false)]
    pub verbose: bool,
    /// Attach an image (local file path) to the prompt. May be repeated.
    /// The file is read and embedded as a base64 data URI and sent to a
    /// vision-capable model. Put `--image` before the prompt text so the
    /// trailing prompt arg does not swallow it.
    #[arg(long = "image", global = true, value_name = "PATH")]
    pub image: Vec<String>,
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
    /// Run the TUI inside a tmux session that survives SSH disconnect.
    /// `ts` has the short alias `rs`: `rs -l`/`rs --list` lists managed tmux
    /// sessions **globally** (every workdir, each with its live workdir path)
    /// plus stopped sessions registered in the central ts registry (`ts.db`),
    /// `rs -r <id>` globally reattaches or cold-starts a stopped one in its
    /// recorded workdir, `rs -c` globally
    /// cleans stopped ts-owned sessions across all workdirs, and `rs -d <id>`
    /// removes one exact global session. A bare `ts`/`rs` **always
    /// creates a fresh session**; resume an existing one with `ts -r <id>`.
    #[command(alias = "rs")]
    Ts {
        /// List all sessions: live managed tmux sessions from every workdir
        /// (path from tmux) plus ts-registered stopped sessions from the registry.
        #[arg(short, long, conflicts_with_all = ["resume", "clean", "delete"])]
        list: bool,
        /// Globally resume/attach by id (live: attach; stopped: cold-start in
        /// its recorded workdir). Accepts a unique displayed id prefix, a full
        /// `opencode-<id>`/bare id, or `$index`.
        #[arg(short, long, conflicts_with_all = ["list", "clean", "delete"])]
        resume: Option<String>,
        /// Globally delete stopped ts-owned sessions no longer running in tmux.
        #[arg(short, long, default_value_t = false, conflicts_with_all = ["list", "resume", "delete"])]
        clean: bool,
        /// Remove one global tmux session and its ts-owned Store record. Accepts
        /// the unique id prefix shown by `ts -l`, a full id, or live `$index`.
        #[arg(short = 'd', long = "delete", value_name = "ID", conflicts_with_all = ["list", "resume", "clean"])]
        delete: Option<String>,
    },
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
        #[arg(short, long, conflicts_with = "continue_")]
        session: Option<String>,
        /// Resume the most recent remote session.
        #[arg(long, default_value_t = false, conflicts_with = "session")]
        continue_: bool,
        /// Interrupt (cancel) the running drain on the resolved session, then
        /// exit. Requires --session <id> or --continue; no prompt needed.
        #[arg(long, default_value_t = false)]
        interrupt: bool,
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
    /// Durable parent-workflow orchestration over focused TODO sessions.
    Todos {
        #[command(subcommand)]
        sub: TodosSub,
    },
    /// Detect and install the optional tools dependencies (tmux). Runs the
    /// embedded `install-skills-dep.sh` with inherited stdio (a sudo password
    /// may be required), then re-seeds the dep-gated skills. Ported from the
    /// former TUI `/install_tools` slash command.
    InstallTools,
    /// Self-update: clone latest main, rebuild, and swap the PATH binary.
    /// Runs a built-in prompt through the headless agent so the agent itself
    /// performs the clone/build/replace steps (handling the busy case).
    Update,
}

#[derive(Subcommand, Debug)]
pub enum ConfigSub {
    /// Show the merged config as JSON.
    Show,
    /// Set the global default model and persist it to opencoder.json.
    Set {
        /// Model as "provider/model_id" (e.g. "anthropic/claude-3" or "glm-5.2").
        model: String,
    },
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

#[derive(Subcommand, Debug)]
pub enum TodosSub {
    /// Validate a prepared TodoSpec without creating sessions or calling a model.
    Validate {
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
    },
    /// Create and execute a prepared TodoSpec workflow.
    Run {
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
        /// Dump a rebuildable filesystem projection after every transition.
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Resume a suspended or interrupted workflow from the Store.
    Resume {
        id: String,
        #[arg(long, default_value_t = false)]
        debug: bool,
    },
    /// Show the canonical workflow projection.
    Show {
        id: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show append-only workflow transition events.
    Events {
        id: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List recent workflows for this workdir.
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Persistently suspend a workflow and cancel active work on next poll.
    Interrupt { id: String },
}

/// Path used to sink TUI logs so they never corrupt the alternate screen.
/// `<data_local_dir>/opencoder/tui.log`. Returns `None` if the data dir is
/// unavailable; the caller then falls back to a temp file (never stdout).
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
/// (e.g. the "WARN stream finished early" line). Headless commands pass `None`,
/// in which case logging goes to a best-effort temp file -- never stdout/stderr
/// -- so the subscriber can never corrupt a terminal regardless of context.
pub fn init_logging(verbose: bool, file_sink: Option<&Path>) {
    let default_filter = if verbose {
        "debug"
    } else {
        "opencoder=info,warn"
    };
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));

    let file = file_sink.and_then(|p| std::fs::File::create(p).ok());
    // Only attempt the temp fallback when the primary sink is unavailable, so
    // the happy path never creates a stray file in the temp dir.
    let temp = file
        .is_none()
        .then(|| std::fs::File::create(fallback_log_path()).ok())
        .flatten();
    let dest = log_dest(file.is_some(), temp.is_some());
    let writer: Box<dyn std::io::Write + Send> = match dest {
        LogDest::PrimaryFile => Box::new(file.expect("log_dest guarantees file is Some")),
        LogDest::TempFallback => Box::new(temp.expect("log_dest guarantees temp is Some")),
        LogDest::Discard => Box::new(std::io::sink()),
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(writer))
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init();
}

/// Best-effort fallback log file when no primary sink was provided/usable.
fn fallback_log_path() -> PathBuf {
    std::env::temp_dir().join("opencoder-tui.log")
}

/// Which log writer strategy to use -- pure decision extracted for testability.
/// Crucially this type can represent ONLY non-tty destinations: stdout/stderr
/// are deliberately unrepresentable so the subscriber can never pollute a
/// terminal (alt-screen corruption).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LogDest {
    PrimaryFile,
    TempFallback,
    Discard,
}

/// Pure decision: given whether the primary file and the temp fallback were
/// opened successfully, pick the best non-tty destination.
pub(crate) fn log_dest(file_ok: bool, temp_ok: bool) -> LogDest {
    if file_ok {
        LogDest::PrimaryFile
    } else if temp_ok {
        LogDest::TempFallback
    } else {
        LogDest::Discard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dest_prefers_primary_file() {
        assert_eq!(log_dest(true, true), LogDest::PrimaryFile);
        assert_eq!(log_dest(true, false), LogDest::PrimaryFile);
    }

    #[test]
    fn log_dest_falls_back_to_temp_when_no_primary() {
        assert_eq!(log_dest(false, true), LogDest::TempFallback);
    }

    #[test]
    fn log_dest_discards_only_when_both_unavailable() {
        assert_eq!(log_dest(false, false), LogDest::Discard);
    }
}
