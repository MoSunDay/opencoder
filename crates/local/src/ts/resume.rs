//! Resolution of `ts -r <target>`: which session the user named, and where a
//! stopped one cold-starts. Pure in its inputs -- the live tmux sessions, the
//! read-only registry rows and the legacy import as a closure -- so the order
//! between import and prefix resolution is testable without tmux or the real
//! data root. `actions::ts_resume` attaches or spawns what this resolves.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use opencoder_store::TsRecord;

use crate::Cli;

use super::actions::resolve_managed_id;
use super::tmux::ManagedSession;

/// What `ts -r <target>` resolved to: a live tmux session to attach to, or a
/// stopped registry session to cold-start in its durable workdir.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResumeTarget {
    Live(String),
    Stopped { id: String, workdir: PathBuf },
}

/// Resolve what `ts -r <target>` names from the live tmux sessions and the
/// read-only registry rows (`registry` is `None` when no registry exists yet).
///
/// A live session is attached as is: the registry writer is never taken for
/// it, not even for the import. A stopped one is found in the registry rows by
/// exact id or prefix. Without a registry yet there are no rows, and the
/// one-time legacy import is the only way to find the session, so once the
/// target is known not to be live, `legacy_import` (a short-lived writer the
/// caller drops before anything is spawned) runs and the target is resolved a
/// second time against what it imported: that resolution is the one that
/// expands a prefix only those rows know and refuses one naming several.
pub(super) async fn resume_target<F, Fut>(
    cli: &Cli,
    target: &str,
    tmux: &[ManagedSession],
    registry: Option<Vec<TsRecord>>,
    legacy_import: F,
) -> Result<ResumeTarget>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<TsRecord>>>,
{
    let (mut records, registry_absent) = match registry {
        Some(records) => (records, false),
        None => (Vec::new(), true),
    };
    let live = |id: &str| {
        tmux.iter()
            .find(|managed| managed.id() == Some(id))
            .map(|managed| managed.name.clone())
    };
    let mut id = resolve_managed_id(target, tmux, &records)?;
    if registry_absent && live(&id).is_none() {
        records = legacy_import().await?;
        // Resolve again against the imported rows: a prefix only they know
        // expands, and one naming several stays ambiguous right here.
        id = resolve_managed_id(target, tmux, &records)?;
    }
    if let Some(name) = live(&id) {
        return Ok(ResumeTarget::Live(name));
    }
    let Some(record) = records.iter().find(|record| record.id == id) else {
        bail!("no global tmux session matching `{target}`. Run `opencoder ts -l` to list.");
    };
    let workdir = resume_workdir(cli, &id, record).await?;
    Ok(ResumeTarget::Stopped { id, workdir })
}

/// The workdir a stopped session cold-starts in: an explicit `--workdir` (which
/// must own the session) or the durable one recorded at registration.
async fn resume_workdir(cli: &Cli, id: &str, record: &TsRecord) -> Result<PathBuf> {
    match (&cli.workdir, &record.workdir) {
        (Some(explicit), _) => {
            let explicit_dir = opencoder_core::data_dir_for(explicit);
            if record.store_dir.as_deref() != Some(explicit_dir.as_path()) {
                bail!("--workdir does not own global tmux session `{id}`");
            }
            tokio::fs::canonicalize(explicit)
                .await
                .with_context(|| format!("resolve --workdir: {}", explicit.display()))
        }
        (None, Some(recorded)) => Ok(recorded.clone()),
        (None, None) => bail!(
            "global tmux session `{id}` predates workdir tracking; resume once with --workdir <original-path>"
        ),
    }
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
