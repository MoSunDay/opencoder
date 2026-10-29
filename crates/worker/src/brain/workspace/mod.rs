//! One node-owned workspace per Brain run; activation directories are evidence history.
use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::{Assignment, ExecutionKind};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

pub(crate) async fn prepare(worker: &Worker, assignment: &Assignment) -> Result<Value> {
    let input = &assignment.request.input;
    if assignment.request.kind == ExecutionKind::Brain {
        let request = super::v4::state::parse_request(input)?;
        if !request.plan.shared_workspace {
            return Ok(Value::Null);
        }
        let root = worker
            .inner
            .layout
            .workspace_dir(ExecutionKind::Brain, &assignment.index.id)?;
        std::fs::create_dir_all(&root)?;
        freeze(
            &root.join("inputs.json"),
            &serde_json::to_value(&request.inputs)?,
        )?;
        freeze(
            &root.join("plan.json"),
            &serde_json::to_value(&request.plan)?,
        )?;
        return Ok(
            json!({"root":root,"run_id":assignment.index.id,"owner_node_id":assignment.index.node_id}),
        );
    }
    if input["brain_workspace"] != true {
        return Ok(Value::Null);
    }
    ensure!(
        assignment.request.kind == ExecutionKind::Operator,
        "shared workspace requires an Operator"
    );
    let parent = &input["brain_layered"];
    let run_id = parent["run_id"]
        .as_str()
        .context("shared workspace parent missing")?;
    let root = {
        let journal = worker.inner.journal.lock().await;
        let record = journal
            .get(run_id)
            .await
            .context("load shared workspace owner record")?
            .context("shared workspace owner must be local")?;
        ensure!(
            record.assignment.index.kind == ExecutionKind::Brain,
            "invalid workspace owner"
        );
        ensure!(
            record.assignment.index.node_id == assignment.index.node_id,
            "workspace node mismatch"
        );
        resolve(&record)?.context("root did not admit a shared workspace")?
    };
    let snapshot = super::v4::state::load(worker, run_id)
        .await
        .with_context(|| format!("load layered state for run {run_id}"))?;
    let operation = snapshot
        .operations
        .iter()
        .find(|op| op.execution_id == assignment.index.id)
        .context("execution is not a persisted operation of this Brain")?;
    ensure!(
        operation.operation_id == parent["operation_id"].as_str().unwrap_or("")
            && Some(operation.activation) == parent["activation"].as_u64()
            && operation.node_id == parent["node_id"].as_str().unwrap_or(""),
        "shared workspace operation mismatch"
    );
    ensure!(
        operation.execution_kind == ExecutionKind::Operator,
        "invalid shared workspace operation kind"
    );
    let step = root.join("steps").join(&assignment.index.id);
    ensure!(
        opencoder_core::fleet::valid_id(&assignment.index.id),
        "invalid step identity"
    );
    checked(&root, &step)?;
    std::fs::create_dir_all(&step)
        .with_context(|| format!("create shared step dir {}", step.display()))?;
    let context = step.join("context.json");
    let metadata = json!({"schema_version":1,"run_id":run_id,"owner_node_id":assignment.index.node_id,
        "root":root,"step":step,"context":context,"activation":operation.activation,
        "layer":operation.layer,"round":operation.round,"node_id":operation.node_id,
        "execution_id":assignment.index.id,"inputs":input["layered_inputs"]});
    freeze(&context, &metadata)?;
    Ok(metadata)
}

pub(crate) fn freeze(path: &Path, value: &Value) -> Result<()> {
    use std::io::Write;
    ensure!(!path.is_symlink(), "workspace metadata cannot be a symlink");
    if !path.exists() {
        let temporary = path.with_extension(format!("{}.tmp", ulid::Ulid::new()));
        let mut file = opencoder_core::platform::fs::create_private_file(&temporary)
            .with_context(|| format!("create private metadata file {}", temporary.display()))?;
        file.write_all(&serde_json::to_vec_pretty(value)?)
            .with_context(|| format!("write metadata file {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("sync metadata file {}", temporary.display()))?;
        let published = opencoder_core::platform::fs::publish_new(&temporary, path);
        if let Err(error) = std::fs::remove_file(&temporary) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        match published {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context(format!("publish metadata file {}", path.display())));
            }
        }
        opencoder_core::platform::fs::sync_directory(
            path.parent().context("metadata parent missing")?,
        )
        .with_context(|| "sync metadata parent directory".to_string())?;
    }
    let existing: Value = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("read frozen metadata {}", path.display()))?,
    )?;
    ensure!(existing == *value, "frozen workspace metadata changed");
    Ok(())
}

/// Resolve only persisted worker annotations, never caller-supplied filesystem paths.
pub(crate) fn resolve(record: &Record) -> Result<Option<PathBuf>> {
    let Some(metadata) = record
        .annotations
        .get("brain_workspace")
        .filter(|v| !v.is_null())
    else {
        return Ok(None);
    };
    let root = PathBuf::from(
        metadata["root"]
            .as_str()
            .context("workspace root missing")?,
    );
    ensure!(root.is_absolute(), "workspace root must be absolute");
    checked(&root, &root)?;
    ensure!(
        root.is_dir(),
        "shared workspace missing; restore the owning node's data"
    );
    Ok(Some(root))
}

pub(crate) fn workdir(record: &Record) -> Result<Option<PathBuf>> {
    let Some(root) = resolve(record)? else {
        return Ok(None);
    };
    let Some(step) = record.annotations["brain_workspace"]["step"].as_str() else {
        return Ok(Some(root));
    };
    let step = PathBuf::from(step);
    checked(&root, &step)?;
    ensure!(
        step.is_dir(),
        "shared workspace activation directory missing"
    );
    Ok(Some(step))
}

pub(crate) fn envs(record: &Record) -> Result<BTreeMap<String, String>> {
    let mut envs = BTreeMap::new();
    if let Some(root) = resolve(record)? {
        let metadata = &record.annotations["brain_workspace"];
        envs.insert(
            "OPENCODER_RUN_WORKSPACE".into(),
            root.to_string_lossy().into(),
        );
        for (key, field) in [
            ("OPENCODER_RUN_CONTEXT", "context"),
            ("OPENCODER_RUN_STEP_DIR", "step"),
        ] {
            if let Some(value) = metadata[field].as_str() {
                envs.insert(key.into(), value.into());
            }
        }
    }
    Ok(envs)
}

pub(crate) fn checked(root: &Path, path: &Path) -> Result<()> {
    ensure!(path.starts_with(root), "path escaped shared workspace");
    let mut cursor = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, std::path::Component::ParentDir),
            "invalid workspace path"
        );
        cursor.push(part);
        // Drive prefixes and roots cannot be symlinks, and a Windows
        // verbatim prefix (`\\?\C:`) is not probeable until the first
        // real segment is appended, so only normal segments are checked.
        if !matches!(part, std::path::Component::Normal(_)) {
            continue;
        }
        crate::layout::reject_symlink(&cursor, "shared workspace path")
            .with_context(|| format!("verify shared workspace component {}", cursor.display()))?;
    }
    Ok(())
}
