//! Durable node admission state. A drained node stays frozen across restart.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const VERSION: u8 = 1;
const FILE_NAME: &str = "admission.json";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Open,
    Frozen,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    mode: Mode,
}

pub(crate) struct AdmissionState {
    path: PathBuf,
    open: AtomicBool,
}

impl AdmissionState {
    pub(crate) fn load(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join(FILE_NAME);
        crate::migration_io::reject_symlink(&path, "node admission state")?;
        let mode = match std::fs::read(&path) {
            Ok(bytes) => {
                let record: Record = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parse {}", path.display()))?;
                anyhow::ensure!(
                    record.version == VERSION,
                    "unsupported admission state version"
                );
                record.mode
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Mode::Open,
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        Ok(Self {
            path,
            open: AtomicBool::new(matches!(mode, Mode::Open)),
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    pub(crate) fn freeze(&self) -> Result<()> {
        self.persist(Mode::Frozen)?;
        self.open.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub(crate) fn reopen(&self) -> Result<()> {
        self.persist(Mode::Open)?;
        self.open.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn persist(&self, mode: Mode) -> Result<()> {
        crate::migration_io::reject_symlink(&self.path, "node admission state")?;
        let parent = self
            .path
            .parent()
            .context("admission state has no parent")?;
        let temp = parent.join(format!(".admission.tmp-{}", ulid::Ulid::new()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .with_context(|| format!("create {}", temp.display()))?;
            file.write_all(&serde_json::to_vec(&Record {
                version: VERSION,
                mode,
            })?)?;
            file.sync_all()?;
            std::fs::rename(&temp, &self.path)?;
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_state_survives_restart_and_reopen_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let state = AdmissionState::load(dir.path()).unwrap();
        assert!(state.is_open());
        state.freeze().unwrap();
        assert!(!AdmissionState::load(dir.path()).unwrap().is_open());
        state.reopen().unwrap();
        assert!(AdmissionState::load(dir.path()).unwrap().is_open());
    }

    #[cfg(unix)]
    #[test]
    fn admission_state_symlink_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, b"outside").unwrap();
        let data = dir.path().join("node");
        std::fs::create_dir(&data).unwrap();
        symlink(&outside, data.join(FILE_NAME)).unwrap();
        let error = AdmissionState::load(&data).err().unwrap().to_string();
        assert!(error.contains("symlink"), "{error}");
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    }
}
