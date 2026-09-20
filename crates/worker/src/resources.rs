//! NFS resource snapshotting for agent execution.
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

#[path = "resources/snapshot.rs"]
mod snapshot;
pub(crate) use snapshot::pin;

/// WASM-only DAGs have no Agent resource dependency. Their modules are pinned
/// separately; copying every prompt, skill and tool would block admission on
/// unrelated files. Unknown definitions and declared agent pins fail closed.
pub fn requires_agent_pool(assignment: &opencoder_core::fleet::Assignment) -> bool {
    use opencoder_core::fleet::ExecutionKind;
    if assignment.request.kind != ExecutionKind::Dag
        || assignment.request.input["_brain"]["action"]["agent_manifests"]
            .as_object()
            .is_some_and(|pins| !pins.is_empty())
    {
        return true;
    }
    !assignment
        .definition
        .as_ref()
        .and_then(|value| opencoder_dag::decode_spec(value.get("spec").unwrap_or(value)).ok())
        .is_some_and(|spec| {
            spec.steps
                .iter()
                .all(|step| matches!(step.kind.executable(), opencoder_dag::StepKind::Wasm { .. }))
        })
}

pub(crate) fn check_mount(path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let path = path
        .canonicalize()
        .context("configured agent resource mount unavailable")?;
    #[cfg(target_os = "linux")]
    {
        let mounts = std::fs::read_to_string("/proc/self/mountinfo")?;
        let mount = mounts
            .lines()
            .filter_map(|line| {
                let (left, right) = line.split_once(" - ")?;
                let fields: Vec<_> = left.split_whitespace().collect();
                let point = PathBuf::from(fields.get(4)?.replace("\\040", " "));
                if !path.starts_with(&point) {
                    return None;
                }
                Some((
                    point.components().count(),
                    fields.get(5)?.to_string(),
                    right.split_whitespace().next()?.to_string(),
                ))
            })
            .max_by_key(|(length, _, _)| *length);
        if !mount.is_some_and(|(_, options, kind)| {
            kind.starts_with("nfs") && options.split(',').any(|o| o == "ro")
        }) {
            bail!("agent.agents_dir must point to a mounted read-only NFS export");
        }
        std::fs::read_dir(path)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!(
            "read-only NFS mount verification for {} requires Linux /proc/self/mountinfo",
            path.display()
        );
    }
    Ok(())
}
