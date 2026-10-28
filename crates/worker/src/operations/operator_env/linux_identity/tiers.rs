//! Access-tier extras for operator executions: the shared read-only group,
//! group read grants over node-owned roots and the read-only tree freeze.

use super::{account_lock, LinuxIdentity};
use opencoder_core::config::OperatorAccessMode;
use crate::layout::DirectoryLayout;
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::ExecutionKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Node-level supplementary group granting read access to shared
/// read-only roots. Created once per node; deliberately never deleted.
pub const READ_ONLY_GROUP: &str = "opencoder-ro";

/// Ownership and mode for one path of the execution environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathOwnership {
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

pub fn owned(uid: u32, gid: u32, mode: u32) -> PathOwnership {
    PathOwnership { uid, gid, mode }
}

/// Profile→ownership mapping result for the operator environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileOwnership {
    pub home: PathOwnership,
    pub workspace: PathOwnership,
    pub snapshot: PathOwnership,
}

/// Profile→ownership mapping (the profile is the frozen config's
/// `operator.access` tier): ReadWrite hands home/workspace to the
/// execution user (0700) and keeps the frozen snapshot node-owned (0440);
/// ReadOnly keeps everything node-owned and non-writable.
pub fn profile_ownership(
    profile: OperatorAccessMode,
    identity: &LinuxIdentity,
    node_uid: u32,
    node_gid: u32,
) -> ProfileOwnership {
    let (executor, node) = match profile {
        OperatorAccessMode::ReadWrite => (
            owned(identity.uid, identity.gid, 0o700),
            owned(node_uid, node_gid, 0o440),
        ),
        OperatorAccessMode::ReadOnly => (
            owned(node_uid, node_gid, 0o555),
            owned(node_uid, node_gid, 0o440),
        ),
    };
    ProfileOwnership {
        home: executor,
        workspace: executor,
        snapshot: node,
    }
}

/// Read-only tree modes: traversable/readable, never writable.
const DIR_MODE: u32 = 0o555;
const FILE_MODE: u32 = 0o444;

static READ_ONLY_GID: OnceLock<u32> = OnceLock::new();

/// Gid of an existing group from `/etc/group`; `None` when absent.
fn group_gid(name: &str) -> Option<u32> {
    let passwd = std::fs::read_to_string("/etc/group").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next() == Some(name))
            .then(|| fields.nth(1)?.parse().ok())
            .flatten()
    })
}

/// Ensure the shared read-only group exists and return its gid. The
/// account-database mutation is serialized with provisioning; the resolved
/// gid is cached so the node pays for `groupadd` at most once per process.
pub fn ensure_read_only_group_gid() -> Result<u32> {
    if let Some(gid) = READ_ONLY_GID.get() {
        return Ok(*gid);
    }
    let _guard = account_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(gid) = READ_ONLY_GID.get() {
        return Ok(*gid);
    }
    if group_gid(READ_ONLY_GROUP).is_none() {
        let groupadd = super::locate_binary("groupadd").context("groupadd binary not found")?;
        super::run(&groupadd, &["-f", READ_ONLY_GROUP])
            .with_context(|| format!("ensure group {READ_ONLY_GROUP}"))?;
    }
    let gid = group_gid(READ_ONLY_GROUP)
        .with_context(|| format!("group {READ_ONLY_GROUP} missing after groupadd"))?;
    let _ = READ_ONLY_GID.set(gid);
    Ok(gid)
}

/// Pure: the node-owned roots a read-only operator session still has to
/// read — its pinned resource snapshot and the operator skill pool.
pub fn read_only_roots(layout: &DirectoryLayout, id: &str) -> Vec<PathBuf> {
    let kind = ExecutionKind::Operator;
    let mut roots = vec![layout.resources_dir(kind, id).unwrap_or_default()];
    roots.push(
        crate::operations::operator_config::dir(layout).join("skills"),
    );
    roots.retain(|root| !root.as_os_str().is_empty());
    roots
}

/// Pure: grant targets must stay inside the node data tree — never the
/// filesystem root itself nor user homes.
pub fn grantable_root(path: &Path) -> bool {
    use std::path::Component;
    let mut normal = path.components().filter_map(|component| match component {
        Component::Normal(part) => Some(part),
        _ => None,
    });
    let Some(first) = normal.next() else {
        return false;
    };
    if normal.next().is_none() {
        return false;
    }
    !matches!(
        first.to_str(),
        Some("home") | Some("root") | Some("Users")
    )
}

fn grant_one(path: &Path, metadata: &std::fs::Metadata, gid: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    let bits = if metadata.is_dir() { 0o050 } else { 0o040 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | bits))
        .with_context(|| format!("grant group read on {}", path.display()))?;
    super::chown_path_gid(path, metadata, gid)
}

/// Recursively hand a root to the read-only group: chgrp plus `g+rX`
/// (directories gain `rx`, plain files `r`). Missing roots are skipped
/// silently; unreachable/unsafe roots are rejected.
pub fn grant_group_read(gid: u32, roots: &[PathBuf]) -> Result<()> {
    for root in roots {
        ensure!(
            grantable_root(root),
            "read-only grant target {} is outside the node data tree",
            root.display()
        );
        let metadata = match std::fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        grant_tree(root, &metadata, gid)?;
    }
    Ok(())
}

fn grant_tree(path: &Path, metadata: &std::fs::Metadata, gid: u32) -> Result<()> {
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    grant_one(path, metadata, gid)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)
            .with_context(|| format!("read {}", path.display()))?
        {
            let entry = entry?;
            let metadata = entry.metadata()?;
            grant_tree(&entry.path(), &metadata, gid)?;
        }
    }
    Ok(())
}

fn freeze_one(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if metadata.is_dir() { DIR_MODE } else { FILE_MODE };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("freeze {} read-only", path.display()))
}

/// Freeze a materialized tree readable-but-not-writable for everyone but
/// the node user: directories 0555, files 0444 (symlinks untouched).
pub fn freeze_read_only_tree(root: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    freeze_one(root, &metadata)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(root)? {
            freeze_read_only_tree(&entry?.path())?;
        }
    }
    Ok(())
}

/// Private `TMPDIR` backing directory for read-write executions: created
/// operator-owned (via the ownership pass) with 0700, idempotently.
pub fn prepare_private_tmp(home: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = home.join("tmp");
    opencoder_core::share_fs::durable_create_dir_all(&tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("mode 0700 for {}", tmp.display()))
}

/// Pure: the env tier's TMPDIR requirement — only read-write executions
/// get a writable scratch directory.
pub fn wants_tmpdir(access: opencoder_core::config::OperatorAccessMode) -> bool {
    matches!(access, opencoder_core::config::OperatorAccessMode::ReadWrite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grantable_root_rejects_root_and_user_homes() {
        assert!(grantable_root(Path::new("/data00/operator/x/resources")));
        assert!(grantable_root(Path::new("/srv/node/data/skills")));
        assert!(!grantable_root(Path::new("/")));
        assert!(!grantable_root(Path::new("/home")));
        assert!(!grantable_root(Path::new("/home/user/data")));
        assert!(!grantable_root(Path::new("/root/data")));
        assert!(!grantable_root(Path::new("/Users/admin/data")));
        assert!(!grantable_root(Path::new("")));
    }

    #[test]
    fn read_only_roots_cover_resources_and_the_operator_skill_pool() {
        let layout = DirectoryLayout::new(Path::new("/data00").to_path_buf(), None).unwrap();
        let roots = read_only_roots(&layout, "operator-01A");
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/data00/operator/operator-01A/resources"),
                PathBuf::from("/data00/operator-config/skills"),
            ]
        );
    }

    #[test]
    fn only_read_write_tiers_get_a_tmpdir() {
        assert!(wants_tmpdir(opencoder_core::config::OperatorAccessMode::ReadWrite));
        assert!(!wants_tmpdir(opencoder_core::config::OperatorAccessMode::ReadOnly));
    }
}
