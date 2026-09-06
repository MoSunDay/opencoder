use anyhow::{Context, Result};
use opencoder_core::fleet::{ExecutionCommand, NodeView};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

const STATE_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionMode {
    Open,
    Frozen,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PersistedState {
    version: u8,
    mode: AdmissionMode,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            mode: AdmissionMode::Open,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AdmissionSnapshot {
    pub mode: AdmissionMode,
    pub inflight_admissions: u64,
}

pub struct AdmissionGate {
    path: PathBuf,
    state: Mutex<PersistedState>,
    transition: Mutex<()>,
    inflight: Arc<AtomicU64>,
}

impl AdmissionGate {
    pub fn load(path: PathBuf) -> Result<Self> {
        let state = load_state(&path)?;
        Ok(Self {
            path,
            state: Mutex::new(state),
            transition: Mutex::new(()),
            inflight: Arc::new(AtomicU64::new(0)),
        })
    }

    pub async fn snapshot(&self) -> AdmissionSnapshot {
        let state = self.state.lock().await;
        AdmissionSnapshot {
            mode: state.mode,
            inflight_admissions: self.inflight.load(Ordering::SeqCst),
        }
    }

    pub async fn enter(&self) -> Result<AdmissionPermit, &'static str> {
        let state = self.state.lock().await;
        if state.mode != AdmissionMode::Open {
            return Err("server admission is frozen");
        }
        self.inflight.fetch_add(1, Ordering::SeqCst);
        Ok(AdmissionPermit {
            inflight: Arc::clone(&self.inflight),
        })
    }

    pub async fn node_allowed(&self, node: &NodeView) -> bool {
        let state = self.state.lock().await;
        state.mode == AdmissionMode::Open
            && node.online
            && node
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.ready)
    }

    pub async fn is_open(&self) -> bool {
        self.state.lock().await.mode == AdmissionMode::Open
    }

    pub async fn transition(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.transition.lock().await
    }

    pub async fn freeze(&self, placement: &Mutex<()>) -> Result<()> {
        let _placement = placement.lock().await;
        let mut state = self.state.lock().await;
        let frozen = PersistedState {
            version: STATE_VERSION,
            mode: AdmissionMode::Frozen,
        };
        persist_state(&self.path, &frozen)?;
        *state = frozen;
        Ok(())
    }

    pub async fn reopen(&self, placement: &Mutex<()>) -> Result<()> {
        let _placement = placement.lock().await;
        let mut state = self.state.lock().await;
        if state.mode == AdmissionMode::Open {
            return Ok(());
        }
        let open = PersistedState {
            version: STATE_VERSION,
            mode: AdmissionMode::Open,
        };
        persist_state(&self.path, &open)?;
        *state = open;
        Ok(())
    }
}

pub struct AdmissionPermit {
    inflight: Arc<AtomicU64>,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        self.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

fn load_state(path: &Path) -> Result<PersistedState> {
    reject_symlink(path)?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedState::default());
        }
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let state: PersistedState =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    anyhow::ensure!(
        state.version == STATE_VERSION,
        "unsupported admission state version"
    );
    Ok(state)
}

fn persist_state(path: &Path, state: &PersistedState) -> Result<()> {
    let temporary = path
        .parent()
        .context("admission state has no parent")?
        .join(format!(".admission.tmp-{}", ulid::Ulid::new()));
    persist_state_at(path, &temporary, state)
}

fn persist_state_at(path: &Path, temporary: &Path, state: &PersistedState) -> Result<()> {
    reject_symlink(path)?;
    let parent = path.parent().context("admission state has no parent")?;
    std::fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(state)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn reject_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            anyhow::bail!("admission state cannot be a symlink: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

pub fn command_requires_admission(command: &ExecutionCommand) -> bool {
    match command.action.as_str() {
        "resume" | "plan" | "execute" | "prompt" | "steer" | "queue" => true,
        "http" => {
            let method = command.input["method"].as_str().unwrap_or("GET");
            let tail = command.input["tail"].as_str().unwrap_or("");
            session_http_requires_admission(method, tail)
        }
        _ => false,
    }
}

pub fn maintenance_requires_admission(command: &ExecutionCommand) -> bool {
    matches!(command.action.as_str(), "ask" | "configure")
}

fn session_http_requires_admission(method: &str, tail: &str) -> bool {
    if method != "POST" {
        return false;
    }
    let path = tail.split_once('?').map_or(tail, |(path, _)| path);
    matches!(path, "fork" | "prompt" | "compact" | "handoff")
        || path
            .strip_prefix("subagents/")
            .is_some_and(|rest| rest.ends_with("/steer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn freeze_survives_restart_until_explicit_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admission.json");
        let placement = Mutex::new(());
        let gate = AdmissionGate::load(path.clone()).unwrap();
        let permit = gate.enter().await.unwrap();
        gate.freeze(&placement).await.unwrap();
        assert_eq!(gate.snapshot().await.inflight_admissions, 1);
        drop(permit);
        assert!(gate.enter().await.is_err());

        let restarted = AdmissionGate::load(path.clone()).unwrap();
        assert_eq!(restarted.snapshot().await.mode, AdmissionMode::Frozen);
        restarted.reopen(&placement).await.unwrap();
        let reopened = AdmissionGate::load(path).unwrap().snapshot().await;
        assert_eq!(reopened.mode, AdmissionMode::Open);
    }

    #[test]
    fn corrupt_or_unknown_state_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admission.json");
        std::fs::write(&path, b"{bad").unwrap();
        assert!(AdmissionGate::load(path.clone()).is_err());
        std::fs::write(&path, br#"{"version":2,"mode":"open"}"#).unwrap();
        assert!(AdmissionGate::load(path).is_err());
    }

    #[test]
    fn unknown_fields_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admission.json");
        std::fs::write(&path, br#"{"version":1,"mode":"open","extra":true}"#).unwrap();
        assert!(AdmissionGate::load(path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn state_symlink_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, b"outside").unwrap();
        let path = dir.path().join("admission.json");
        symlink(&outside, &path).unwrap();
        assert!(AdmissionGate::load(path).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn preexisting_temp_symlink_is_not_followed_or_removed() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admission.json");
        let temporary = dir.path().join(".admission.tmp-fixed");
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, &temporary).unwrap();
        let state = PersistedState {
            version: STATE_VERSION,
            mode: AdmissionMode::Frozen,
        };
        assert!(persist_state_at(&path, &temporary, &state).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
        assert!(std::fs::symlink_metadata(temporary)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn only_new_execution_inputs_require_admission() {
        let command = |action: &str, input| ExecutionCommand {
            action: action.into(),
            input,
        };
        for action in ["resume", "plan", "execute", "prompt", "steer", "queue"] {
            assert!(command_requires_admission(&command(
                action,
                serde_json::Value::Null
            )));
        }
        assert!(command_requires_admission(&command(
            "http",
            serde_json::json!({"method":"POST","tail":"prompt?delivery=steer"}),
        )));
        assert!(command_requires_admission(&command(
            "http",
            serde_json::json!({"method":"POST","tail":"subagents/child/steer"}),
        )));
        for tail in ["interrupt", "cancel", "questions/call-1/answer"] {
            assert!(!command_requires_admission(&command(
                "http",
                serde_json::json!({"method":"POST","tail":tail}),
            )));
        }
        assert!(!command_requires_admission(&command(
            "http",
            serde_json::json!({"method":"GET","tail":"messages"}),
        )));
        assert!(maintenance_requires_admission(&command(
            "ask",
            serde_json::Value::Null,
        )));
        assert!(!maintenance_requires_admission(&command(
            "status",
            serde_json::Value::Null,
        )));
    }
}
