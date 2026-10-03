//! NFS exports are unavailable in the native Windows operator.
use serde::Serialize;
use std::{net::SocketAddr, path::PathBuf};

pub struct NfsServerOpts {
    pub export_root: PathBuf,
    pub host: String,
    pub port: u16,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NfsServerStatus {
    pub running: bool,
    pub host: String,
    pub port: u16,
    pub read_only: bool,
    pub export_root: String,
}

pub struct NfsServerHandle {
    never: std::convert::Infallible,
}
impl NfsServerHandle {
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        match self.never {}
    }
    pub fn shutdown(self) {
        match self.never {}
    }
}
pub fn spawn_nfs_server(_: &NfsServerOpts) -> anyhow::Result<NfsServerHandle> {
    anyhow::bail!("NFS exports require Linux or macOS; Windows supports TUI and operator")
}
pub fn nfs_status(handle: Option<&NfsServerHandle>) -> NfsServerStatus {
    if let Some(handle) = handle {
        match handle.never {}
    }
    NfsServerStatus {
        running: false,
        host: "127.0.0.1".into(),
        port: 2049,
        read_only: true,
        export_root: String::new(),
    }
}
pub fn default_opts_from_config(config: &opencoder_core::config::Config) -> NfsServerOpts {
    let nfs = &config.agent.nfs;
    NfsServerOpts {
        export_root: config
            .agent
            .agents_dir
            .clone()
            .or_else(opencoder_core::agent::agents_dir)
            .unwrap_or_default(),
        host: nfs.host.clone(),
        port: nfs.port,
        read_only: nfs.read_only,
    }
}
