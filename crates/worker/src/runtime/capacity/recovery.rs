//! Startup recovery runs only after container cleanup and durable journal recovery.
use super::HostCapacity;
use crate::{journal::Journal, lifecycle::StopIntent};
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use std::path::Path;

impl HostCapacity {
    pub(crate) async fn verify_recovery(&self, data: &Path) -> Result<()> {
        if !self
            .store
            .runtime_tickets(&self.runtime_id)
            .await?
            .iter()
            .any(|(_, _, phase)| phase == "running")
        {
            return Ok(());
        }
        self.verify_owners(data).await
    }

    pub(crate) async fn recover(
        &self,
        data: &Path,
        node: &str,
        journal: &mut Journal,
    ) -> Result<()> {
        let tickets: Vec<_> = self
            .store
            .runtime_tickets(&self.runtime_id)
            .await?
            .into_iter()
            .filter(|(_, _, phase)| phase == "running")
            .collect();
        if tickets.is_empty() {
            return Ok(());
        }
        // Validate the complete set before changing any reservation. Unknown
        // records must use explicit offline recovery, never a blanket release.
        for (ticket, execution, _) in &tickets {
            let record = journal
                .records
                .get(execution)
                .context("capacity owner record missing")?;
            ensure!(
                record.assignment.index.kind != ExecutionKind::Brain,
                "Brain has unresolved running capacity; use storage settle-brain-crash for the exact root"
            );
            ensure!(
                record.assignment.index.node_id == node
                    && record.assignment.index.id == *execution
                    && record
                        .queue
                        .as_ref()
                        .and_then(|queue| queue.ticket.as_deref())
                        == Some(ticket),
                "capacity ticket differs from its execution journal"
            );
            ensure!(
                record.assignment.index.status != ExecutionStatus::Running
                    && record.assignment.index.status != ExecutionStatus::Cancelling,
                "execution recovery is not durable"
            );
        }
        self.verify_owners(data).await?;
        for (ticket, execution, _) in tickets {
            if journal.records[&execution].assignment.index.status == ExecutionStatus::Pending {
                journal.request_stop(&execution, StopIntent::Interrupt, false)?;
            }
            self.store
                .recover_capacity(&ticket, &execution, &self.runtime_id)
                .await?;
        }
        Ok(())
    }

    async fn verify_owners(&self, data: &Path) -> Result<()> {
        ensure!(
            opencoder_session::process::active_owned_processes() == 0,
            "runtime still owns processes; capacity recovery refused"
        );
        let runtime = self
            .store
            .runtimes()
            .await?
            .into_iter()
            .find(|runtime| runtime.id == self.runtime_id)
            .context("capacity recovery requires a registered Runtime")?;
        let registered = runtime.config["data_dir"]
            .as_str()
            .context("Runtime data directory missing")?;
        ensure!(
            std::fs::canonicalize(registered)? == data,
            "capacity recovery Runtime directory differs"
        );
        let unit = runtime.config["unit"]
            .as_str()
            .context("Runtime service identity missing")?;
        ensure!(
            unit.starts_with("opencoder-runtime-")
                && unit.ends_with(".service")
                && unit
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@".contains(&byte)),
            "capacity recovery requires an exact managed Runtime service"
        );
        verify_kernel_owners(unit)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn verify_kernel_owners(unit: &str) -> Result<()> {
    let current = std::process::id();
    let own = std::fs::read_to_string("/proc/self/cgroup")?;
    ensure!(
        belongs_to(&own, unit),
        "Runtime process is outside its registered service"
    );
    let remaining = kernel_owners(Path::new("/proc"), unit, current)?;
    ensure!(
        remaining.is_empty(),
        "Runtime still has kernel process owners: {remaining:?}"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn verify_kernel_owners(_: &str) -> Result<()> {
    anyhow::bail!(
        "capacity recovery requires verified kernel process ownership; use offline recovery"
    )
}

#[cfg(any(target_os = "linux", test))]
fn belongs_to(cgroup: &str, unit: &str) -> bool {
    cgroup.lines().any(|row| {
        row.splitn(3, ':')
            .nth(2)
            .is_some_and(|path| path.split('/').any(|part| part == unit))
    })
}

#[cfg(any(target_os = "linux", test))]
fn kernel_owners(proc_root: &Path, unit: &str, current: u32) -> Result<Vec<u32>> {
    let mut remaining = Vec::new();
    for entry in std::fs::read_dir(proc_root)? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == current {
            continue;
        }
        let group = match std::fs::read_to_string(entry.path().join("cgroup")) {
            Ok(group) => group,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if belongs_to(&group, unit) {
            remaining.push(pid);
        }
    }
    Ok(remaining)
}

#[cfg(test)]
mod tests;
