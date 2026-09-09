//! Persisted node limits; lowering the limit never interrupts running work.
use anyhow::{Context, Result};
use opencoder_core::fleet::NodeScheduling;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

pub(crate) struct SchedulingState {
    path: PathBuf,
    value: Mutex<NodeScheduling>,
}

impl SchedulingState {
    pub(crate) fn load(data: &Path, max_runs: usize) -> Result<Self> {
        let path = data.join("scheduling.json");
        crate::migration_io::reject_symlink(&path, "node scheduling")?;
        let value = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("parse node scheduling")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => NodeScheduling {
                max_runs,
                queue_order: Default::default(),
            },
            Err(e) => return Err(e).context("read node scheduling"),
        };
        value.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            path,
            value: Mutex::new(value),
        })
    }
    pub(crate) fn get(&self) -> NodeScheduling {
        *self.value.lock().unwrap()
    }
    pub(crate) fn save(&self, value: NodeScheduling) -> Result<()> {
        value.validate().map_err(anyhow::Error::msg)?;
        crate::migration_io::reject_symlink(&self.path, "node scheduling")?;
        opencoder_core::atomic_write(&self.path, &serde_json::to_vec(&value)?)?;
        std::fs::File::open(self.path.parent().context("scheduling parent")?)?.sync_all()?;
        *self.value.lock().unwrap() = value;
        Ok(())
    }
}
