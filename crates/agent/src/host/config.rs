use anyhow::{ensure, Result};
use opencoder_core::fleet::NodeRegistration;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Immutable per-runtime launch metadata stored alongside its release bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub endpoint: String,
    pub data_dir: PathBuf,
    pub unit: String,
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.endpoint)?;
        ensure!(
            url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port().is_some(),
            "runtime endpoint must be explicit loopback HTTP"
        );
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "invalid runtime URL"
        );
        ensure!(
            self.data_dir.is_absolute()
                && !self
                    .data_dir
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "runtime data directory must be absolute"
        );
        ensure!(
            self.unit.starts_with("opencoder-runtime-")
                && self.unit.ends_with(".service")
                && self
                    .unit
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b)),
            "invalid runtime unit"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub runtime_id: Option<String>,
    pub build: serde_json::Value,
    pub owned_processes: usize,
    pub registration: NodeRegistration,
    pub snapshot: opencoder_core::fleet::NodeSnapshot,
    pub indexes: Vec<opencoder_core::fleet::ExecutionIndex>,
    pub can_hibernate: bool,
}
