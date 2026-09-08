//! `opencoder-cli` — remote management CLI for `opencoder-server`.
//!
//! Wraps the control plane's full HTTP API (health/drain, executions,
//! sessions relay, nodes, DAG, TODO, project, brain, teams, custom agents)
//! plus a `raw` escape hatch that can drive any route verbatim. Stdout is a
//! single JSON document per invocation; human notes go to stderr. Auth is
//! plain Bearer, same token the server and agent binaries use.

pub mod cmd;
pub mod ctx;
pub mod http;
pub mod out;
pub mod sse;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::ctx::Ctx;

#[derive(Parser, Debug)]
#[command(
    name = "opencoder-cli",
    version,
    long_version = opencoder_core::version::VERSION_LONG,
    about = "opencoder fleet control-plane CLI: every opencoder-server API over Bearer auth"
)]
pub struct Cli {
    /// Print machine-readable version, commit and fleet protocol metadata.
    #[arg(long)]
    build_info: bool,
    /// Server base URL (e.g. http://127.0.0.1:8080); env OPENCODER_SERVER_URL.
    #[arg(long, global = true)]
    server: Option<String>,
    /// Bearer token. Mutually exclusive with --token-file.
    #[arg(long, global = true, conflicts_with = "token_file")]
    token: Option<String>,
    /// Read the Bearer token from a credential file.
    #[arg(long, global = true, value_name = "PATH")]
    token_file: Option<PathBuf>,
    /// Human-facing notes on stderr (request lines, interrupt info).
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Cluster health probe.
    Health,
    /// Readiness probe (honors the frozen drain mode).
    Ready,
    /// Server time (auth-protected clock).
    Time,
    /// Drain admin: status / freeze / reopen.
    #[command(subcommand)]
    Drain(DrainAction),
    /// Executions: list, create, inspect, command, events, artifacts, ...
    #[command(subcommand)]
    Exec(cmd::executions::ExecCmd),
    /// Sessions (via the control relay): prompt, fork, compact, ...
    #[command(subcommand)]
    Session(cmd::sessions::SessionCmd),
    /// Nodes, models, skills and node-local task admin.
    #[command(subcommand)]
    Nodes(cmd::nodes::NodesCmd),
    /// DAG definitions, dispatch and runs.
    #[command(subcommand)]
    Dag(cmd::dag::DagCmd),
    /// TODO environments, tools, templates and workflow runs.
    #[command(subcommand)]
    Todo(cmd::todo::TodoCmd),
    /// Project tracking: goals, milestones, todos, runs.
    #[command(subcommand)]
    Project(cmd::project::ProjectCmd),
    /// Brain capabilities, search, plans and dispatch.
    #[command(subcommand)]
    Brain(cmd::brain::BrainCmd),
    /// Team definitions.
    #[command(subcommand)]
    Teams(cmd::teams::TeamsCmd),
    /// Versioned custom agents: cards, resources, NFS export.
    #[command(subcommand)]
    Agents(cmd::agents::AgentCmd),
    /// Escape hatch: any method + path against the server, verbatim.
    #[command(subcommand)]
    Raw(cmd::raw::RawCmd),
}

#[derive(Subcommand, Debug)]
pub enum DrainAction {
    /// Read the open/frozen status and drained aggregate.
    Status,
    /// Freeze admission (server + nodes).
    Freeze,
    /// Reopen admission.
    Reopen,
}

pub async fn run(cli: Cli) -> anyhow::Result<i32> {
    if cli.build_info {
        println!("{}", opencoder_core::version::build_info_json());
        return Ok(0);
    }
    let Some(command) = cli.command else {
        out::note("no command given; see `opencoder-cli --help`");
        return Ok(64);
    };
    let ctx: Ctx = ctx::resolve(
        cli.server.as_deref(),
        cli.token.as_deref(),
        cli.token_file.clone(),
        cli.verbose,
    )?;
    match command {
        Command::Health => cmd::system::health(&ctx).await,
        Command::Ready => cmd::system::ready(&ctx).await,
        Command::Time => cmd::system::time(&ctx).await,
        Command::Drain(action) => cmd::system::drain(&ctx, action).await,
        Command::Exec(sub) => cmd::executions::run(&ctx, sub).await,
        Command::Session(sub) => cmd::sessions::run(&ctx, sub).await,
        Command::Nodes(sub) => cmd::nodes::run(&ctx, sub).await,
        Command::Dag(sub) => cmd::dag::run(&ctx, sub).await,
        Command::Todo(sub) => cmd::todo::run(&ctx, sub).await,
        Command::Project(sub) => cmd::project::run(&ctx, sub).await,
        Command::Brain(sub) => cmd::brain::run(&ctx, sub).await,
        Command::Teams(sub) => cmd::teams::run(&ctx, sub).await,
        Command::Agents(sub) => cmd::agents::run(&ctx, sub).await,
        Command::Raw(sub) => cmd::raw::run(&ctx, sub).await,
    }
}
