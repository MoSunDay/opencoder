//! Host-wide slots shared by all release runtimes on this machine.
use anyhow::{ensure, Result};
use opencoder_store::fleet::FleetStore;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct HostBinding {
    pub database: PathBuf,
    pub runtime_id: String,
}

pub(crate) struct HostCapacity {
    pub store: Arc<FleetStore>,
    pub runtime_id: String,
}

impl HostCapacity {
    pub async fn load(data_dir: &Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(data_dir.join("host-binding.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let binding: HostBinding = serde_json::from_slice(&bytes)?;
        ensure!(
            binding.database.is_absolute(),
            "host database must be absolute"
        );
        let store = Arc::new(FleetStore::open(&binding.database).await?);
        store.capacity().await?;
        let tickets = store.runtime_tickets(&binding.runtime_id).await?;
        if tickets.iter().any(|(_, _, phase)| phase == "running") {
            #[cfg(target_os = "linux")]
            managed_runtime_is_alone(Path::new("/proc"), std::process::id())?;
            #[cfg(not(target_os = "linux"))]
            anyhow::bail!("running capacity recovery requires explicit process cleanup proof");
            recover_running_tickets(
                &store,
                &binding.runtime_id,
                opencoder_session::process::active_owned_processes(),
            )
            .await?;
        }
        Ok(Some(Self {
            store,
            runtime_id: binding.runtime_id,
        }))
    }
}

/// The caller holds the node lock, has cleaned its containers, and has proved
/// that its managed service has no other kernel process owners.
async fn recover_running_tickets(
    store: &FleetStore,
    runtime: &str,
    owned_processes: usize,
) -> Result<()> {
    ensure!(
        owned_processes == 0,
        "runtime has owned processes; capacity recovery is unsafe"
    );
    for (ticket, execution, phase) in store.runtime_tickets(runtime).await? {
        if phase == "running" {
            store.recover_capacity(&ticket, &execution, runtime).await?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn managed_runtime_is_alone(proc_root: &Path, pid: u32) -> Result<()> {
    let current = fs::read_to_string(proc_root.join(pid.to_string()).join("cgroup"))?;
    let scope = current
        .lines()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .find_map(|path| {
            let mut prefix = String::new();
            for part in path.split('/').filter(|part| !part.is_empty()) {
                prefix.push('/');
                prefix.push_str(part);
                if part.starts_with("opencoder-runtime-") && part.ends_with(".service") {
                    return Some(prefix);
                }
            }
            None
        })
        .ok_or_else(|| anyhow::anyhow!("capacity recovery requires a managed Runtime service"))?;
    let descendant = format!("{scope}/");
    for entry in fs::read_dir(proc_root)? {
        let entry = entry?;
        let Some(owner) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if owner == pid {
            continue;
        }
        let cgroups = match fs::read_to_string(entry.path().join("cgroup")) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        ensure!(
            !cgroups
                .lines()
                .filter_map(|line| line.splitn(3, ':').nth(2))
                .any(|path| path == scope || path.starts_with(&descendant)),
            "Runtime still has kernel process owner {owner}; capacity recovery is unsafe"
        );
    }
    Ok(())
}

impl crate::Worker {
    pub fn runtime_id(&self) -> Option<&str> {
        self.inner
            .host_capacity
            .as_ref()
            .map(|host| host.runtime_id.as_str())
    }
    pub(crate) async fn reconcile_capacity(&self) -> Result<()> {
        let Some(host) = &self.inner.host_capacity else {
            return Ok(());
        };
        let journal = self.inner.journal.lock().await;
        for (ticket, id, phase) in host.store.runtime_tickets(&host.runtime_id).await? {
            if phase == "queued"
                && !journal.records.get(&id).is_some_and(|r| {
                    r.queue.as_ref().and_then(|q| q.ticket.as_deref()) == Some(&ticket)
                        && r.assignment.index.status
                            == opencoder_core::fleet::ExecutionStatus::Pending
                })
            {
                host.store
                    .finish_capacity(&ticket, &host.runtime_id)
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn finish_slot(&self, ticket: Option<&str>) -> Result<()> {
        if let (Some(host), Some(ticket)) = (&self.inner.host_capacity, ticket) {
            host.store.finish_capacity(ticket, &host.runtime_id).await?;
        }
        Ok(())
    }

    /// Retirement never freezes the queue. A runtime may sleep only after
    /// execution futures, tools, persistence and its accepted queue are empty.
    pub async fn can_hibernate(&self) -> bool {
        self.inner.active.lock().await.is_empty()
            && self.inner.tasks.active_count() == 0
            && self
                .inner
                .pending_runs
                .load(std::sync::atomic::Ordering::SeqCst)
                == 0
            && opencoder_session::process::active_owned_processes() == 0
            && self.inner.persistence_error.lock().unwrap().is_none()
            && self
                .inner
                .state
                .project
                .require()
                .is_ok_and(|p| p.persistence_error.lock().unwrap().is_none())
            && crate::brain::outbox::frames(self)
                .await
                .is_ok_and(|frames| frames.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::fleet::FleetStore;

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_restart_requires_the_only_process_in_its_service_scope() {
        let root = tempfile::tempdir().unwrap();
        for (pid, group) in [
            (1, "0::/system.slice/opencoder-runtime-test.service\n"),
            (2, "0::/system.slice/unrelated.service\n"),
        ] {
            fs::create_dir(root.path().join(pid.to_string())).unwrap();
            fs::write(root.path().join(pid.to_string()).join("cgroup"), group).unwrap();
        }
        managed_runtime_is_alone(root.path(), 1).unwrap();
        fs::write(
            root.path().join("2/cgroup"),
            "0::/system.slice/opencoder-runtime-test.service/children\n",
        )
        .unwrap();
        assert!(managed_runtime_is_alone(root.path(), 1)
            .unwrap_err()
            .to_string()
            .contains("kernel process owner 2"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unmanaged_restart_cannot_release_capacity() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("1")).unwrap();
        fs::write(
            root.path().join("1/cgroup"),
            "0::/user.slice/session.scope\n",
        )
        .unwrap();
        assert!(managed_runtime_is_alone(root.path(), 1)
            .unwrap_err()
            .to_string()
            .contains("managed Runtime service"));
    }

    #[tokio::test]
    async fn restart_recovery_releases_running_tickets_when_no_process_is_owned() {
        let store = FleetStore::open_memory().await.unwrap();
        store.initialize_capacity(1).await.unwrap();
        store
            .enqueue_capacity("ticket", "execution", "runtime")
            .await
            .unwrap();
        assert!(store.claim_capacity("ticket", "runtime").await.unwrap());

        recover_running_tickets(&store, "runtime", 0).await.unwrap();

        assert!(store.runtime_tickets("runtime").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restart_recovery_fails_closed_with_owned_processes() {
        let store = FleetStore::open_memory().await.unwrap();
        store.initialize_capacity(1).await.unwrap();
        store
            .enqueue_capacity("ticket", "execution", "runtime")
            .await
            .unwrap();
        assert!(store.claim_capacity("ticket", "runtime").await.unwrap());

        let error = recover_running_tickets(&store, "runtime", 1)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("owned processes"));
        assert_eq!(store.capacity().await.unwrap().running, 1);
    }
}
