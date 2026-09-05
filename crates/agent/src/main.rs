//! `opencoder-agent` — the fleet worker binary.
//!
//! Registers to a central `opencoder-server`, claims prompt tasks AND DAG
//! workflow runs, and executes them locally: agent steps through the real
//! session runner, python steps through the embedded RustPython VM (or an
//! `runc` container), artifacts under the node-local `workflow_root`. The
//! VM/runc dependency chain lives ONLY here — the main `opencoder` binary
//! and `opencoder-server` never link it.
//!
//! Node (client) token semantics are inherited from the node crate: the
//! token must be RESOLVED by the caller (`--token` else
//! `OPENCODER_SERVER_TOKEN`); a worker never auto-generates one.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "opencoder-agent",
    version,
    about = "opencoder fleet worker: prompt tasks + node-side DAG workflow execution"
)]
struct Args {
    #[command(subcommand)]
    command: Option<AgentCommand>,
    /// Server base URL (e.g. http://127.0.0.1:8080). Required for `run`.
    #[arg(long)]
    remote: Option<String>,
    /// Bearer token: --token, then OPENCODER_SERVER_TOKEN. Never generated.
    #[arg(long)]
    token: Option<String>,
    /// Friendly unique node name override; defaults to a hostname-derived
    /// label with a short process-local suffix.
    #[arg(long)]
    name: Option<String>,
    /// Directory the agent operates from (config discovery + workdir).
    #[arg(long)]
    workdir: Option<PathBuf>,
    /// Root for node-local workflow artifacts (`/workflow/<run_id>/...`).
    #[arg(long, default_value = "/workflow")]
    workflow_root: PathBuf,
    /// Disable DAG workflow claiming (plain prompt-task worker).
    #[arg(long)]
    no_dag: bool,
}

#[derive(Subcommand, Debug)]
enum AgentCommand {
    /// Run the agent loop (default when no subcommand is given).
    Run,
    /// Node-side DAG tooling (offline: no server, store, or LLM wiring).
    Dag {
        #[command(subcommand)]
        command: DagCommand,
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

/// Resolve the worker bearer token: `--token` flag, then the
/// `OPENCODER_SERVER_TOKEN` environment variable. A node NEVER
/// auto-generates (an invented token could never authenticate against the
/// remote server) — same contract as `opencoder_cli::daemon::resolve_client_token`.
fn resolve_token(flag: Option<String>) -> Result<String> {
    if let Some(t) = flag {
        return Ok(t);
    }
    std::env::var("OPENCODER_SERVER_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .context("agent token required: pass --token or set OPENCODER_SERVER_TOKEN")
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging();

    // Offline tooling short-circuits BEFORE the server/token/store/LLM
    // wiring below: `dag prepare-rootfs` only touches the local filesystem.
    if let Some(AgentCommand::Dag {
        command: DagCommand::PrepareRootfs { out },
    }) = &args.command
    {
        return prepare_rootfs(out);
    }

    let remote = args
        .remote
        .clone()
        .context("agent requires --remote <server-base-url>")?;
    let token = resolve_token(args.token.clone())?;
    let workdir = args
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let name = args.name.clone().unwrap_or_else(|| {
        // Same derivation as the old `opencode daemon --client` default.
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "opencoder-agent".into())
    });

    // DAG extension: build the agent-binary hook (uplink + local store +
    // LLM client + config) unless --no-dag. Failures are fatal — a worker
    // that cannot open its store or config cannot serve prompt tasks either.
    let dag: Option<std::sync::Arc<dyn opencoder_node::DagHook>> = if args.no_dag {
        None
    } else {
        Some(std::sync::Arc::new(
            build_dag_hook(&remote, &token, &workdir, args.workflow_root.clone()).await?,
        ))
    };

    let opts = opencoder_node::NodeOpts {
        name,
        remote,
        token,
        workdir,
        heartbeat_interval: opencoder_node::DEFAULT_HEARTBEAT_INTERVAL,
        claim_interval: opencoder_node::DEFAULT_CLAIM_INTERVAL,
        version: env!("CARGO_PKG_VERSION").to_string(),
        local_store_dir: None,
        dag,
    };
    opencoder_node::run_node(opts, None).await
}

/// Adapter wiring the node crate's [`opencoder_node::DagHook`] onto the
/// real DAG scheduling loop. A plain data record (no behavior of its own):
/// `claim` is one signed GET; `execute` assembles [`opencoder_dag_runtime::RunDeps`]
/// and lets `execute_run` own the whole lifecycle (it reports the terminal
/// status itself, so run-level `error` outcomes are still `Ok(())` here).
struct DagRuntimeHook {
    uplink: std::sync::Arc<opencoder_node::uplink::Uplink>,
    store: std::sync::Arc<dyn opencoder_store::Store>,
    client: std::sync::Arc<dyn opencoder_llm::ChatStream>,
    config: opencoder_core::Config,
    workdir: PathBuf,
    workflow_root: PathBuf,
}

async fn build_dag_hook(
    remote: &str,
    token: &str,
    workdir: &std::path::Path,
    workflow_root: PathBuf,
) -> Result<DagRuntimeHook> {
    let uplink = std::sync::Arc::new(opencoder_node::uplink::Uplink::new(remote, token)?);
    // Same local-store rule as the node runner (data_dir_for on workdir):
    // a server and an agent sharing one machine share one store.
    let store_dir = opencoder_core::data_dir_for(workdir);
    tokio::fs::create_dir_all(&store_dir).await.ok();
    let store: std::sync::Arc<dyn opencoder_store::Store> = std::sync::Arc::new(
        opencoder_store::LibsqlStore::open(store_dir.join("opencoder.db")).await?,
    );
    let config = opencoder_core::Config::load(workdir)?;
    let client = build_chat_client(&config)?;
    Ok(DagRuntimeHook {
        uplink,
        store,
        client,
        config,
        workdir: workdir.to_path_buf(),
        workflow_root,
    })
}

/// Real LLM backend from the resolved config (same construction as the node
/// runner's `build_default_client` / `cli/src/run.rs`).
fn build_chat_client(
    config: &opencoder_core::Config,
) -> Result<std::sync::Arc<dyn opencoder_llm::ChatStream>> {
    let ep = config.resolve_endpoint()?;
    let client = opencoder_llm::ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )?;
    Ok(std::sync::Arc::new(client))
}

#[async_trait::async_trait]
impl opencoder_node::DagHook for DagRuntimeHook {
    async fn claim(
        &self,
        node_id: &str,
    ) -> anyhow::Result<Option<opencoder_dag::protocol::DagClaimedRun>> {
        self.uplink.dag_claim(node_id).await
    }

    async fn execute(
        &self,
        run: opencoder_dag::protocol::DagClaimedRun,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        let deps = opencoder_dag_runtime::RunDeps {
            uplink: std::sync::Arc::clone(&self.uplink),
            exec: opencoder_dag_runtime::ExecDeps {
                store: std::sync::Arc::clone(&self.store),
                client: std::sync::Arc::clone(&self.client),
                workdir: self.workdir.clone(),
                config: self.config.clone(),
            },
            workflow_root: self.workflow_root.clone(),
        };
        let status = opencoder_dag_runtime::execute_run(deps, run, cancel_rx).await?;
        tracing::info!(status = %status, "dag run hook finished");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_flag_wins_over_flag_path() {
        assert_eq!(
            resolve_token(Some("explicit".into())).unwrap(),
            "explicit",
            "flag path must short-circuit before env lookup"
        );
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
}
