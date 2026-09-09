use anyhow::{bail, Result};
use opencoder_core::fleet::{valid_id, ExecutionKind};
use std::path::{Path, PathBuf};

/// Pure path policy for node-owned execution data.
///
/// New executions live below `<data>/<kind>/<id>`. The legacy roots are
/// retained only so an existing execution can keep using its original tree
/// until an operator runs the explicit layout migration.
#[derive(Clone, Debug)]
pub struct DirectoryLayout {
    root: PathBuf,
    legacy_workflow_root: PathBuf,
}

/// Every kind with a node-owned journal root. `load_current` rebuilds the
/// in-memory journal from these directories on restart, so the list must
/// stay in lockstep with `kind_root` writers — a missing kind silently
/// drops its records (and their queued work) across a node restart.
pub(crate) const ALL_KINDS: [ExecutionKind; 8] = [
    ExecutionKind::Agent,
    ExecutionKind::Dag,
    ExecutionKind::Team,
    ExecutionKind::Todos,
    ExecutionKind::Project,
    ExecutionKind::Maintenance,
    ExecutionKind::System,
    ExecutionKind::Operator,
];

pub(crate) fn reserved_execution_entry(name: &str) -> bool {
    matches!(
        name,
        "execution.json" | "migration.json" | "resources" | "team"
    )
}

impl DirectoryLayout {
    pub fn new(root: PathBuf, legacy_workflow_root: Option<PathBuf>) -> Result<Self> {
        if !root.is_absolute() {
            bail!("node data directory must be absolute");
        }
        let legacy_workflow_root = legacy_workflow_root.unwrap_or_else(|| root.join("workflow"));
        if !legacy_workflow_root.is_absolute() {
            bail!("legacy workflow directory must be absolute");
        }
        Ok(Self {
            root,
            legacy_workflow_root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kind_root(&self, kind: ExecutionKind) -> PathBuf {
        self.root.join(kind.prefix())
    }

    pub(crate) fn checked_kind_root(&self, kind: ExecutionKind) -> Result<PathBuf> {
        self.contained(self.kind_root(kind))
    }

    pub fn execution_dir(&self, kind: ExecutionKind, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        self.contained(self.kind_root(kind).join(id))
    }

    pub fn record_path(&self, kind: ExecutionKind, id: &str) -> Result<PathBuf> {
        self.contained(self.execution_dir(kind, id)?.join("execution.json"))
    }

    pub(crate) fn migration_receipt_path(&self, kind: ExecutionKind, id: &str) -> Result<PathBuf> {
        self.contained(self.execution_dir(kind, id)?.join("migration.json"))
    }

    pub fn resources_dir(&self, kind: ExecutionKind, id: &str) -> Result<PathBuf> {
        self.contained(self.execution_dir(kind, id)?.join("resources"))
    }

    pub fn team_state_dir(&self, kind: ExecutionKind, id: &str) -> Result<PathBuf> {
        self.contained(self.execution_dir(kind, id)?.join("team"))
    }

    pub(crate) fn legacy_record_path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        self.contained(self.root.join("executions").join(format!("{id}.json")))
    }

    pub(crate) fn legacy_records_root(&self) -> PathBuf {
        self.root.join("executions")
    }

    pub(crate) fn legacy_resources_dir(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        self.contained(self.root.join("resources").join(id))
    }

    pub(crate) fn legacy_team_dir(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        self.contained(self.root.join("teams").join(id))
    }

    pub(crate) fn legacy_dag_dir(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        let root = self.checked_legacy_workflow_root()?;
        reject_symlink(&root.join(id), "legacy workflow execution")?;
        Ok(root.join(id))
    }

    pub(crate) fn checked_legacy_workflow_root(&self) -> Result<PathBuf> {
        reject_symlink_chain(&self.legacy_workflow_root, "legacy workflow root")?;
        Ok(self.legacy_workflow_root.clone())
    }

    fn contained(&self, path: PathBuf) -> Result<PathBuf> {
        if !path.starts_with(&self.root) {
            bail!("execution path escaped node data directory");
        }
        let relative = path
            .strip_prefix(&self.root)
            .expect("path prefix checked above");
        let mut cursor = self.root.clone();
        for component in relative.components() {
            cursor.push(component);
            match std::fs::symlink_metadata(&cursor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    bail!("execution path contains a symlink: {}", cursor.display());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(path)
    }
}

pub(crate) fn reject_symlink(path: &Path, label: &str) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("{label} cannot be a symlink: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reject_symlink_chain(path: &Path, label: &str) -> Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component);
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("{label} contains a symlink: {}", cursor.display())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("invalid execution id");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_are_contained_without_prefix_inference() {
        let layout = DirectoryLayout::new(PathBuf::from("/node"), None).unwrap();
        assert_eq!(
            layout
                .record_path(ExecutionKind::Team, "historical_name")
                .unwrap(),
            PathBuf::from("/node/team/historical_name/execution.json")
        );
        assert!(layout
            .record_path(ExecutionKind::Agent, "../escape")
            .is_err());
    }
}
