//! `opencoder-agent` — the fleet worker binary.
//!
//! Registers to `opencoder-server` over an outbound channel and executes
//! agents, teams, workflows, projects and maintenance locally: agent steps through the real
//! session runner, python steps through the embedded RustPython VM (or an
//! `runc` container), artifacts under the node-local typed execution tree. The
//! VM/runc dependency chain lives ONLY here — the main `opencoder` binary
//! and `opencoder-server` never link it.
//!
//! Node (client) token semantics are inherited from the node crate: the
//! token must be supplied by `--token`, `--token-file`, or
//! `OPENCODER_SERVER_TOKEN`; a worker never auto-generates one.

use std::{ffi::OsString, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod storage;

#[derive(Parser, Debug)]
#[command(
    name = "opencoder-agent",
    version,
    long_version = opencoder_core::version::VERSION_LONG,
    about = "opencoder fleet execution node: agents, teams, workflows and maintenance"
)]
struct Args {
    /// Print machine-readable version, commit and fleet protocol metadata.
    #[arg(long)]
    build_info: bool,
    #[command(subcommand)]
    command: Option<AgentCommand>,
    /// Server base URL (e.g. http://127.0.0.1:8080). Required for `run`.
    #[arg(long)]
    remote: Option<String>,
    /// Bearer token. Mutually exclusive with --token-file.
    #[arg(long, conflicts_with = "token_file")]
    token: Option<String>,
    /// Read the Bearer token from a credential file.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,
    /// Friendly unique node name override; defaults to a hostname-derived
    /// label with a short process-local suffix.
    #[arg(long)]
    name: Option<String>,
    /// Directory the agent operates from (config discovery + workdir).
    #[arg(long)]
    workdir: Option<PathBuf>,
    /// Legacy DAG artifact root used when reading or migrating old executions.
    #[arg(long)]
    workflow_root: Option<PathBuf>,
    /// Node-local persistent state; independent of the server and CLI stores.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Concurrent top-level executions (default: available CPU capacity).
    #[arg(long)]
    max_runs: Option<usize>,
    /// Do not accept DAG workflows on this node.
    #[arg(long)]
    no_dag: bool,
}

#[derive(Subcommand, Debug)]
enum AgentCommand {
    /// Run the agent loop (default when no subcommand is given).
    Run,
    /// Internal isolated embedded Python executor; JSON over standard input/output.
    #[command(hide = true)]
    InternalPythonStep,
    /// Internal owner for one external workload and all of its descendants.
    #[command(hide = true)]
    InternalProcessSupervisor {
        #[arg(long, hide = true)]
        runc_root: Option<PathBuf>,
        #[arg(long, hide = true)]
        runc_id: Option<String>,
        #[arg(last = true, required = true, hide = true)]
        command: Vec<OsString>,
    },
    /// Node-side DAG tooling (offline: no server, store, or LLM wiring).
    Dag {
        #[command(subcommand)]
        command: DagCommand,
    },
    /// Offline node-storage maintenance.
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },
}

#[derive(Subcommand, Debug)]
enum DagCommand {
    /// Scaffold the shared read-only rootfs used by `sandbox: runc` python
    /// steps (mount points, resolv.conf copy, provisioning README).
    PrepareRootfs {
        /// Directory to write the rootfs scaffold tree into.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum StorageCommand {
    /// Copy legacy execution trees into the typed directory layout.
    MigrateLayout,
}

/// Resolve an explicitly supplied worker credential without logging it.
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
            "agent token required: pass --token, --token-file, or set OPENCODER_SERVER_TOKEN"
        ),
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,opencoder_agent=debug,opencoder_node=debug")
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// `dag prepare-rootfs`: write the shared-rootfs scaffold and print the
/// created tree. Pure filesystem work — deliberately reachable without
/// any network, store, or LLM setup.
fn prepare_rootfs(out: &std::path::Path) -> Result<()> {
    opencoder_dag_runtime::sandbox::oci::write_rootfs_template(out)
        .with_context(|| format!("write rootfs template under {}", out.display()))?;
    println!("rootfs scaffold written to {}", out.display());
    println!();
    print_tree(out);
    println!();
    println!(
        "next: add a python interpreter under usr/ — see {} for the provisioning guide",
        out.join("README.md").display()
    );
    Ok(())
}

/// Depth-first listing of a freshly created directory tree (children
/// sorted per directory so the output is deterministic).
fn print_tree(root: &std::path::Path) {
    fn walk(dir: &std::path::Path, prefix: &str) {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        let last = names.len().saturating_sub(1);
        for (i, name) in names.iter().enumerate() {
            let tail = i == last;
            println!("{}{}{}", prefix, if tail { "└── " } else { "├── " }, name);
            let path = dir.join(name);
            if path.is_dir() {
                walk(
                    &path,
                    &format!("{}{}", prefix, if tail { "    " } else { "│   " }),
                );
            }
        }
    }
    println!("{}", root.display());
    walk(root, "");
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.build_info {
        println!("{}", opencoder_core::version::build_info_json());
        return Ok(());
    }
    if let Some(AgentCommand::InternalProcessSupervisor {
        runc_root,
        runc_id,
        command,
    }) = &args.command
    {
        let cleanup = match (runc_root, runc_id) {
            (Some(root), Some(id)) => Some(opencoder_session::process::RuncCleanup {
                root: root.clone(),
                id: id.clone(),
            }),
            (None, None) => None,
            _ => anyhow::bail!("runc cleanup requires both root and id"),
        };
        let code = opencoder_session::process::supervisor_main(command.clone(), cleanup)?;
        std::process::exit(code);
    }
    if matches!(&args.command, Some(AgentCommand::InternalPythonStep)) {
        return opencoder_dag_runtime::exec::python::worker_main();
    }
    if args.command.is_none() || matches!(&args.command, Some(AgentCommand::Run)) {
        opencoder_session::process::configure_supervisor_binary(std::env::current_exe()?)?;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(args))
}

async fn run(args: Args) -> Result<()> {
    init_logging();

    // Offline tooling short-circuits BEFORE the server/token/store/LLM
    // wiring below: `dag prepare-rootfs` only touches the local filesystem.
    if let Some(AgentCommand::Dag {
        command: DagCommand::PrepareRootfs { out },
    }) = &args.command
    {
        return prepare_rootfs(out);
    }

    let workdir = args
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let data_dir = args
        .data_dir
        .clone()
        .unwrap_or_else(|| opencoder_core::data_dir_for(&workdir).join("node-v2"));
    if matches!(
        args.command,
        Some(AgentCommand::Storage {
            command: StorageCommand::MigrateLayout
        })
    ) {
        return storage::migrate_layout(&data_dir, args.workflow_root.as_deref());
    }

    let remote = args
        .remote
        .clone()
        .context("agent requires --remote <server-base-url>")?;
    let token = resolve_token(args.token.clone(), args.token_file.clone())?;
    let name = args.name.clone().unwrap_or_else(|| {
        // Same derivation as the old `opencode daemon --client` default.
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "opencoder-agent".into())
    });

    let worker = opencoder_worker::Worker::open(
        opencoder_worker::WorkerOptions {
            name,
            workdir,
            data_dir,
            workflow_root: args.workflow_root,
            max_runs: args.max_runs,
            dag: !args.no_dag,
        },
        None,
    )
    .await?;
    let service: std::sync::Arc<dyn opencoder_node::fleet::NodeService> =
        std::sync::Arc::new(worker.clone());
    let mut fleet =
        tokio::spawn(async move { opencoder_node::fleet::run(&remote, &token, service).await });
    tokio::select! {
        result = &mut fleet => result.context("node channel task failed")?,
        _ = shutdown_signal() => {
            // Keep the channel alive while the frozen snapshot and durable
            // interrupt/cleanup progress remain visible to Server.
            let drained = worker.drain_shutdown().await;
            fleet.abort();
            let _ = fleet.await;
            drained
        },
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_flag_wins_over_flag_path() {
        assert_eq!(
            resolve_token(Some("explicit".into()), None).unwrap(),
            "explicit",
            "flag path must short-circuit before env lookup"
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
            "opencoder-agent",
            "--token",
            "one",
            "--token-file",
            "/run/credentials/token"
        ])
        .is_err());
    }

    #[test]
    fn dag_prepare_rootfs_args_parse() {
        let args = Args::try_parse_from([
            "opencoder-agent",
            "dag",
            "prepare-rootfs",
            "--out",
            "/tmp/x",
        ])
        .expect("dag prepare-rootfs must parse");
        match args.command {
            Some(AgentCommand::Dag {
                command: DagCommand::PrepareRootfs { out },
            }) => assert_eq!(out, PathBuf::from("/tmp/x")),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn storage_migrate_layout_args_parse_without_server() {
        let args = Args::try_parse_from([
            "opencoder-agent",
            "--data-dir",
            "/tmp/node",
            "storage",
            "migrate-layout",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            Some(AgentCommand::Storage {
                command: StorageCommand::MigrateLayout
            })
        ));
    }
}
