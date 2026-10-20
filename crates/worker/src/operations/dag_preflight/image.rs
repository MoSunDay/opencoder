//! Only an explicit maintenance receipt can replace a retained OCI runtime image.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const RECEIPT: &str = "runtime-image-replacement.json";
const RUNNERS: [&str; 2] = ["dag-runner", "agent-step-runner"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    schema: String,
    runtime_data: PathBuf,
    source_rootfs: Vec<PathBuf>,
    target_rootfs: PathBuf,
    runner_sha256: BTreeMap<String, String>,
}

pub(super) fn resolve(
    data: &Path,
    accepted: Option<&Path>,
    configured: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let (Some(accepted), Some(configured)) = (accepted, configured) else {
        return Ok(None);
    };
    if accepted == configured {
        return Ok(None);
    }
    let receipt = configured
        .parent()
        .context("runtime image parent missing")?
        .join(RECEIPT);
    let metadata = match std::fs::symlink_metadata(&receipt) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 16384,
        "runtime image replacement receipt must be a bounded regular file"
    );
    let value: Replacement = serde_json::from_slice(&std::fs::read(&receipt)?)?;
    let data = data.canonicalize()?;
    let target = configured.canonicalize()?;
    let source = accepted.canonicalize()?;
    ensure!(
        value.schema == "opencoder.runtime-image-replacement.v1"
            && value.runtime_data == data
            && value.target_rootfs == target
            && target.starts_with(data.join("programs"))
            && !value.source_rootfs.is_empty()
            && value.source_rootfs.iter().all(|path| path.is_absolute())
            && value.source_rootfs.contains(&source),
        "runtime image replacement does not belong to the accepted image and runtime"
    );
    ensure!(
        value.runner_sha256.len() == RUNNERS.len(),
        "runtime image runner receipt is incomplete"
    );
    for name in RUNNERS {
        let path = target.join("usr/bin").join(name);
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "runtime image runner must be a regular file"
        );
        let actual = format!("{:x}", Sha256::digest(std::fs::read(path)?));
        ensure!(
            value.runner_sha256.get(name) == Some(&actual),
            "runtime image runner changed after maintenance"
        );
    }
    // Native preflight still verifies the full build metadata against this node.
    // Accepted inputs, resources, workspace paths, and historical config stay intact.
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, serde_json::Value) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("dag/rootfs");
        let target = root.path().join("programs/replacement/rootfs");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(target.join("usr/bin")).unwrap();
        let mut hashes = BTreeMap::new();
        for name in RUNNERS {
            std::fs::write(target.join("usr/bin").join(name), name.as_bytes()).unwrap();
            hashes.insert(name, format!("{:x}", Sha256::digest(name.as_bytes())));
        }
        let receipt = json!({"schema":"opencoder.runtime-image-replacement.v1",
            "runtime_data":root.path().canonicalize().unwrap(),
            "source_rootfs":[source.canonicalize().unwrap()],
            "target_rootfs":target.canonicalize().unwrap(),"runner_sha256":hashes});
        (root, source, target, receipt)
    }

    fn write(target: &Path, receipt: &serde_json::Value) {
        std::fs::write(
            target.parent().unwrap().join(RECEIPT),
            serde_json::to_vec(receipt).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn ordinary_configuration_change_never_replaces_a_frozen_image() {
        let (root, source, target, _) = fixture();
        assert_eq!(
            resolve(root.path(), Some(&source), Some(&target)).unwrap(),
            None
        );
    }

    #[test]
    fn explicit_maintenance_selects_only_its_registered_image() {
        let (root, source, target, receipt) = fixture();
        write(&target, &receipt);
        assert_eq!(
            resolve(root.path(), Some(&source), Some(&target)).unwrap(),
            Some(target)
        );
        assert!(source.is_dir());
    }

    #[test]
    fn changed_scope_or_runner_prevents_recovery() {
        let (root, source, target, receipt) = fixture();
        for (key, replacement) in [
            ("runtime_data", json!("/another/runtime")),
            ("target_rootfs", json!("/another/image")),
            ("source_rootfs", json!(["/another/source"])),
            ("runner_sha256", json!({})),
        ] {
            let mut changed = receipt.clone();
            changed[key] = replacement;
            write(&target, &changed);
            assert!(resolve(root.path(), Some(&source), Some(&target)).is_err());
        }
        write(&target, &receipt);
        std::fs::write(target.join("usr/bin/dag-runner"), "changed").unwrap();
        assert!(resolve(root.path(), Some(&source), Some(&target)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_receipt_is_not_a_maintenance_authorization() {
        let (root, source, target, receipt) = fixture();
        let other = root.path().join("other.json");
        std::fs::write(&other, serde_json::to_vec(&receipt).unwrap()).unwrap();
        std::os::unix::fs::symlink(other, target.parent().unwrap().join(RECEIPT)).unwrap();
        assert!(resolve(root.path(), Some(&source), Some(&target)).is_err());
    }
}
