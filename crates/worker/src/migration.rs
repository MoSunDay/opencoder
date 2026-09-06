use crate::{
    journal::{read_record, same_execution, Record},
    migration_io::{
        absolute_path, copy_tree_verified, durable_json, entries_equal, sha256_file, sync_tree,
        trees_equal, Staging,
    },
    DirectoryLayout,
};
use anyhow::{bail, Context, Result};
use opencoder_core::fleet::ExecutionKind;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const RECEIPT_FILE: &str = "migration.json";

#[derive(Debug, Serialize)]
pub struct MigrationReport {
    pub migrated: usize,
    pub already_current: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct MigrationReceipt {
    version: u32,
    legacy_execution_sha256: String,
}

struct LegacyExecution {
    record: Record,
    record_path: PathBuf,
}

pub fn migrate_layout(data_dir: &Path, workflow_root: Option<&Path>) -> Result<MigrationReport> {
    std::fs::create_dir_all(data_dir)?;
    let root = std::fs::canonicalize(data_dir)?;
    let workflow_root = workflow_root.map(absolute_path).transpose()?;
    let layout = DirectoryLayout::new(root.clone(), workflow_root)?;
    let lock = crate::migration_io::open_lock_file(&root.join("node.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|error| anyhow::anyhow!("node data directory already in use: {error}"))?;

    let executions = discover(&layout)?;
    validate_legacy_roots(&layout, executions.keys())?;
    for execution in executions.values() {
        preflight_sources(&layout, execution)?;
        validate_destination(&layout, execution)?;
    }

    let mut report = MigrationReport {
        migrated: 0,
        already_current: 0,
    };
    for execution in executions.values() {
        if migrate_one(&layout, execution)? {
            report.migrated += 1;
        } else {
            report.already_current += 1;
        }
    }
    Ok(report)
}

pub(crate) fn receipt_matches_legacy(
    layout: &DirectoryLayout,
    id: &str,
    kind: ExecutionKind,
) -> Result<bool> {
    let receipt_path = layout.migration_receipt_path(kind, id)?;
    let receipt: MigrationReceipt = match std::fs::read(&receipt_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", receipt_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if receipt.version != 1 {
        bail!("unsupported migration receipt version");
    }
    let legacy = layout.legacy_record_path(id)?;
    if !legacy.is_file() {
        return Ok(false);
    }
    Ok(receipt.legacy_execution_sha256 == sha256_file(&legacy)?)
}

fn discover(layout: &DirectoryLayout) -> Result<BTreeMap<String, LegacyExecution>> {
    let mut out = BTreeMap::new();
    let root = layout.legacy_records_root();
    if !root.exists() {
        return Ok(out);
    }
    if std::fs::symlink_metadata(&root)?.file_type().is_symlink() {
        bail!("legacy execution root cannot be a symlink");
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            bail!("legacy execution records cannot be symlinks");
        }
        let path = entry.path();
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            continue;
        }
        let record = read_record(&path, true)?;
        let id = record.assignment.index.id.clone();
        if path.file_stem().and_then(|part| part.to_str()) != Some(id.as_str()) {
            bail!("legacy journal file name does not match execution id");
        }
        if out
            .insert(
                id.clone(),
                LegacyExecution {
                    record,
                    record_path: path,
                },
            )
            .is_some()
        {
            bail!("duplicate legacy execution {id}");
        }
    }
    Ok(out)
}

fn validate_legacy_roots<'a>(
    layout: &DirectoryLayout,
    ids: impl Iterator<Item = &'a String> + Clone,
) -> Result<()> {
    let known: BTreeSet<&str> = ids.map(String::as_str).collect();
    for (root, workflow) in [
        (layout.root().join("resources"), false),
        (layout.root().join("teams"), false),
        (layout.checked_legacy_workflow_root()?, true),
    ] {
        if !root.exists() {
            continue;
        }
        if std::fs::symlink_metadata(&root)?.file_type().is_symlink() {
            bail!("legacy layout root cannot be a symlink: {}", root.display());
        }
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                bail!("legacy layout roots cannot contain symlink entries");
            }
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                bail!("legacy layout contains a non-UTF8 execution id");
            };
            if known.contains(id.as_str()) {
                continue;
            }
            if workflow && matches!(id.as_str(), "rootfs" | "bundles") {
                continue;
            }
            bail!("legacy data has no execution record: {id}");
        }
    }
    Ok(())
}

fn validate_destination(layout: &DirectoryLayout, source: &LegacyExecution) -> Result<()> {
    let id = &source.record.assignment.index.id;
    let kind = source.record.assignment.index.kind;
    let target_dir = layout.execution_dir(kind, id)?;
    let target = layout.record_path(kind, id)?;
    if !target.exists() && !target_dir.exists() {
        return Ok(());
    }
    if !target.exists() {
        bail!("current execution directory has no execution record for {id}");
    }
    let current = read_record(&target, false)?;
    if serde_json::to_value(&source.record)? == serde_json::to_value(&current)? {
        return verify_existing_trees(layout, source);
    }
    if same_execution(&source.record, &current) && receipt_matches_legacy(layout, id, kind)? {
        return Ok(());
    }
    bail!("current execution conflicts with legacy execution {id}")
}

fn verify_existing_trees(layout: &DirectoryLayout, source: &LegacyExecution) -> Result<()> {
    let id = &source.record.assignment.index.id;
    let kind = source.record.assignment.index.kind;
    let target = layout.execution_dir(kind, id)?;
    let mut expected = BTreeSet::from(["execution.json".to_string()]);
    let resources = layout.legacy_resources_dir(id)?;
    if resources.exists() {
        if !trees_equal(&resources, &layout.resources_dir(kind, id)?)? {
            bail!("current resource tree conflicts with legacy execution {id}");
        }
        expected.insert("resources".into());
    }
    match source.record.assignment.index.kind {
        ExecutionKind::Team | ExecutionKind::System => {
            let team = layout.legacy_team_dir(id)?;
            if team.exists() {
                if !trees_equal(&team, &layout.team_state_dir(kind, id)?)? {
                    bail!("current team tree conflicts with legacy execution {id}");
                }
                expected.insert("team".into());
            }
        }
        ExecutionKind::Dag => {
            let dag = layout.legacy_dag_dir(id)?;
            if dag.exists() {
                for entry in std::fs::read_dir(dag)? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !entries_equal(&entry.path(), &target.join(&name))? {
                        bail!("current DAG tree conflicts with legacy execution {id}");
                    }
                    expected.insert(name);
                }
            }
        }
        _ => {}
    }
    // Symlink rejection is a hard gate: scan every entry first so that
    // directory iteration order can never mask it behind other violations.
    for entry in std::fs::read_dir(&target)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            bail!(
                "current execution contains a symlink: {}",
                entry.path().display()
            );
        }
    }
    for entry in std::fs::read_dir(&target)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != RECEIPT_FILE && !expected.contains(&name) {
            bail!("current execution has data absent from legacy execution {id}");
        }
    }
    Ok(())
}

fn preflight_sources(layout: &DirectoryLayout, source: &LegacyExecution) -> Result<()> {
    let mut destinations = BTreeSet::new();
    for (tree, relative) in source_trees(layout, source)? {
        if !tree.exists() {
            continue;
        }
        validate_source_tree(&tree, &relative, &mut destinations)?;
    }
    Ok(())
}

fn validate_source_tree(
    source: &Path,
    relative: &Path,
    destinations: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if std::fs::symlink_metadata(source)?.file_type().is_symlink() {
        bail!("migration refuses symlink: {}", source.display());
    }
    if !relative.as_os_str().is_empty() && !destinations.insert(relative.to_path_buf()) {
        bail!("legacy source trees collide at {}", relative.display());
    }
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = relative.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("migration refuses symlink: {}", path.display());
        }
        if file_type.is_dir() {
            validate_source_tree(&path, &target, destinations)?;
        } else if file_type.is_file() {
            if !destinations.insert(target.clone()) {
                bail!("legacy source trees collide at {}", target.display());
            }
            sha256_file(&path)?;
        }
    }
    Ok(())
}

fn migrate_one(layout: &DirectoryLayout, source: &LegacyExecution) -> Result<bool> {
    let id = &source.record.assignment.index.id;
    let kind = source.record.assignment.index.kind;
    let target = layout.execution_dir(kind, id)?;
    let record_path = layout.record_path(kind, id)?;
    if record_path.exists() {
        if !receipt_matches_legacy(layout, id, kind)? {
            let receipt = receipt(source)?;
            durable_json(&layout.migration_receipt_path(kind, id)?, &receipt)?;
        }
        return Ok(false);
    }
    let kind_root = layout.kind_root(kind);
    opencoder_core::share_fs::durable_create_dir_all(&kind_root)?;
    let staging = kind_root.join(format!(".{id}.migrate-{}", ulid::Ulid::new()));
    opencoder_core::share_fs::durable_create_dir_all(&staging)?;
    let cleanup = Staging(staging.clone());
    durable_json(&staging.join("execution.json"), &source.record)?;
    for (from, relative) in source_trees(layout, source)? {
        if from.exists() {
            copy_tree_verified(&from, &staging.join(relative))?;
        }
    }
    durable_json(&staging.join(RECEIPT_FILE), &receipt(source)?)?;
    sync_tree(&staging)?;
    std::fs::rename(&staging, &target)?;
    std::fs::File::open(&kind_root)?.sync_all()?;
    drop(cleanup);
    Ok(true)
}

fn source_trees(
    layout: &DirectoryLayout,
    source: &LegacyExecution,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let id = &source.record.assignment.index.id;
    let mut trees = vec![(layout.legacy_resources_dir(id)?, PathBuf::from("resources"))];
    match source.record.assignment.index.kind {
        ExecutionKind::Team | ExecutionKind::System => {
            trees.push((layout.legacy_team_dir(id)?, PathBuf::from("team")));
        }
        ExecutionKind::Dag => {
            let dag = layout.legacy_dag_dir(id)?;
            if dag.exists() {
                for entry in std::fs::read_dir(&dag)? {
                    let entry = entry?;
                    if entry
                        .file_name()
                        .to_str()
                        .is_some_and(crate::layout::reserved_execution_entry)
                    {
                        bail!("legacy DAG uses a reserved execution path");
                    }
                }
            }
            trees.push((dag, PathBuf::new()));
        }
        _ => {}
    }
    Ok(trees)
}

fn receipt(source: &LegacyExecution) -> Result<MigrationReceipt> {
    Ok(MigrationReceipt {
        version: 1,
        legacy_execution_sha256: sha256_file(&source.record_path)?,
    })
}
