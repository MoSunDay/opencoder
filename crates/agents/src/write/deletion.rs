//! Keep the resource writer lock from the reviewed card through removal.
use opencoder_core::agent::{agent_dir, validate_agent_name, AgentMeta};
use std::{fs::File, io, path::PathBuf};

pub struct LockedAgentDeletion {
    _lock: File,
    directory: PathBuf,
    pub meta: AgentMeta,
}

impl LockedAgentDeletion {
    /// Call on a blocking thread, preserving the caller's resource root.
    pub fn prepare(name: &str) -> io::Result<Self> {
        validate_agent_name(name).map_err(crate::io::invalid_input)?;
        let lock = crate::resources::lock::write_lock()?;
        let directory =
            agent_dir(name).ok_or_else(|| crate::io::not_found("agent root missing"))?;
        let meta = crate::resources::filesystem::read_json(&directory.join("meta.json"))?;
        Ok(Self {
            _lock: lock,
            directory,
            meta,
        })
    }

    /// The resource writer lock remains held until the directory is removed.
    pub fn remove(self) -> io::Result<()> {
        std::fs::remove_dir_all(&self.directory)
    }
}
