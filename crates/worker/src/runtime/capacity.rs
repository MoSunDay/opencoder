//! Host-wide slots shared by all release runtimes on this machine.
use anyhow::{ensure, Result};
use opencoder_store::fleet::FleetStore;
use serde::{Deserialize, Serialize};
use std::{
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
        // A reservation is never expired. Restart recovery must prove process
        // cleanup before resolving any running ticket; refuse ambiguous state.
        ensure!(store.runtime_tickets(&binding.runtime_id).await?.iter().all(|(_,_,phase)| phase != "running"),
            "runtime has unresolved running capacity reservations; verify owned process cleanup before recovery");
        Ok(Some(Self {
            store,
            runtime_id: binding.runtime_id,
        }))
    }
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
