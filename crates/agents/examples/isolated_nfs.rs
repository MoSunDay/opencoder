//! Export an immutable, deployment-scoped Agent bundle without a control plane.
//! The caller owns bundle verification and publication before process startup.
use anyhow::{ensure, Context, Result};
use opencoder_agents::{spawn_nfs_server, NfsServerOpts};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().context("usage: isolated_nfs ROOT PORT")?);
    let port: u16 = args
        .next()
        .context("missing loopback port")?
        .to_str()
        .context("port is not UTF-8")?
        .parse()?;
    ensure!(args.next().is_none() && port != 0, "invalid arguments");
    let server = spawn_nfs_server(&NfsServerOpts {
        export_root: root,
        host: "127.0.0.1".into(),
        port,
        read_only: true,
    })?;
    println!(
        "{}",
        serde_json::to_string(&opencoder_agents::nfs_status(Some(&server)))?
    );
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {},
    }
    server.shutdown();
    Ok(())
}
