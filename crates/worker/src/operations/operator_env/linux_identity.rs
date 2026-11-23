//! Per-execution Linux system users for operator executions.
//!
//! Each operator execution owns a dedicated system account (`ocop-*`,
//! nologin shell, no home creation — the operator env owns the directory).
//! The private home/workspace trees belong to that account while the frozen
//! config snapshot stays node-owned, so an execution user can never rewrite
//! its own frozen configuration. Terminal-state release hands produced
//! files back to the node user and deletes the account. Every system
//! mutation fails closed with context-rich errors.

use super::snapshot_path;
use crate::layout::DirectoryLayout;
use anyhow::{anyhow, ensure, Context, Result};
use opencoder_core::config::{LinuxIdentityMode, OperatorAccessMode};
use opencoder_core::platform::RunAsIdentity;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub mod sweep;
pub mod tiers;

/// Linux user names are limited to 32 bytes (`useradd`).
const MAX_USERNAME: usize = 32;
/// ULID suffix folded into the derived user name (12 chars keep every kind
/// prefix below the 32-byte limit).
const SUFFIX_CHARS: usize = 12;

/// A provisioned execution account. Provisioning captures the live ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxIdentity {
    pub username: String,
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
}

impl LinuxIdentity {
    /// Persistable form for harness state (phase 3 drops privileges from it).
    pub fn run_as(&self) -> RunAsIdentity {
        RunAsIdentity {
            uid: self.uid,
            gid: self.gid,
            groups: self.groups.clone(),
            username: Some(self.username.clone()),
        }
    }
}

/// Valid Linux user name: at most 32 bytes, starts with a lowercase letter,
/// then `[a-z0-9-]` only.
pub fn valid_username(name: &str) -> bool {
    name.len() <= MAX_USERNAME
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Derive `ocop-<kind-prefix>-<ulid-suffix>` from an execution id
/// (`<kind-prefix>-<ULID>`). Fails closed on malformed ids.
pub fn username_for(execution_id: &str) -> Result<String> {
    let (prefix, ulid) = execution_id
        .split_once('-')
        .with_context(|| format!("execution id {execution_id:?} is not <prefix>-<ulid>"))?;
    ensure!(
        !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "execution id prefix {prefix:?} must be [a-z0-9]+"
    );
    let suffix: String = ulid
        .chars()
        .rev()
        .take(SUFFIX_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    ensure!(
        !suffix.is_empty(),
        "execution id {execution_id:?} has no ulid"
    );
    let name = format!("ocop-{prefix}-{suffix}").to_ascii_lowercase();
    ensure!(
        valid_username(&name),
        "derived user name {name:?} exceeds the 32-byte/[a-z0-9-] limits"
    );
    Ok(name)
}

/// Search PATH plus the common sbin directories for an executable.
pub fn locate_binary(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for dir in ["/usr/sbin", "/sbin", "/usr/local/sbin"] {
        let dir = PathBuf::from(dir);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.into_iter()
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

pub fn node_uid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn node_gid() -> u32 {
    unsafe { libc::getegid() }
}

/// Fail-closed capability probe: provisioning needs a root node process
/// and a resolvable `useradd`.
pub fn ensure_capability() -> Result<()> {
    ensure!(
        node_uid() == 0,
        "operator linux identity provisioning requires the node process to run as root"
    );
    locate_binary("useradd").map(|_| ()).ok_or_else(|| {
        anyhow!("operator linux identity provisioning requires useradd (searched PATH and sbin directories)")
    })
}

/// Pure admission matrix: `Off` always passes; `Enforced` requires the
/// provisioning capability.
pub fn admission(mode: LinuxIdentityMode, capable: bool) -> Result<()> {
    if matches!(mode, LinuxIdentityMode::Off) {
        return Ok(());
    }
    capable.then_some(()).ok_or_else(|| {
        anyhow!("operator linux identity is enforced but the node cannot provision accounts")
    })
}

/// Admission decision for an operator execution from its config plane.
pub fn admit(config: &opencoder_core::Config) -> Result<()> {
    admission(config.operator.linux_identity, ensure_capability().is_ok())
}

/// useradd/userdel race on the account database; serialize mutations.
fn account_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn run(command: &Path, args: &[&str]) -> Result<String> {
    let name = command
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| command.display().to_string());
    let output = std::process::Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("spawn {name}"))?;
    ensure!(
        output.status.success(),
        "{name} {} failed ({}): {}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Live account lookup by name; `None` when the account is absent.
pub fn lookup(username: &str) -> Option<LinuxIdentity> {
    let id = locate_binary("id")?;
    let uid = run(&id, &["-u", username]).ok()?.parse().ok()?;
    let gid = run(&id, &["-g", username]).ok()?.parse().ok()?;
    let groups = run(&id, &["-G", username])
        .ok()?
        .split_whitespace()
        .map(|group| group.parse().ok())
        .collect::<Option<Vec<u32>>>()?;
    Some(LinuxIdentity {
        username: username.to_string(),
        uid,
        gid,
        groups,
    })
}

/// Provision (idempotently) the dedicated system account for an execution:
/// a system account (`-r`) whose home directory is NOT created (`-M`, the
/// operator env owns it) with a nologin shell. `groups` adds supplementary
/// memberships (the read-only tier's shared group).
pub fn provision(home: &Path, execution_id: &str, groups: &[&str]) -> Result<LinuxIdentity> {
    let _guard = account_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let username = username_for(execution_id)?;
    if let Some(existing) = lookup(&username) {
        return Ok(existing);
    }
    let useradd = locate_binary("useradd").context("useradd binary not found")?;
    let shell = locate_binary("nologin")
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/sbin/nologin".to_string());
    let home = home.to_string_lossy().into_owned();
    let memberships = groups.join(",");
    let mut args: Vec<&str> = vec!["-r", "-M", "-d", &home, "-s", &shell];
    if !memberships.is_empty() {
        args.extend(["-G", &memberships]);
    }
    args.push(&username);
    run(&useradd, &args).with_context(|| format!("provision system user {username}"))?;
    lookup(&username).with_context(|| format!("system user {username} missing after useradd"))
}

/// Terminal-state teardown: hand produced files back to the node user, then
/// delete the account. Idempotent — an absent account or missing
/// directories are success, so a call after a completed release succeeds.
pub fn release(home: &Path, workspace: &Path, execution_id: &str) -> Result<()> {
    let _guard = account_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let username = username_for(execution_id)?;
    release_account(&username, home, workspace)
}

/// Release one `ocop-*` account by name (the stale sweep knows accounts,
/// not execution ids). Caller serializes account-database mutations.
pub(super) fn release_account(username: &str, home: &Path, workspace: &Path) -> Result<()> {
    if lookup(username).is_none() {
        return Ok(());
    }
    for directory in [home, workspace] {
        if directory.is_dir() {
            chown_tree(directory, node_uid(), node_gid())
                .with_context(|| format!("hand {} back to the node user", directory.display()))?;
        }
    }
    let userdel = locate_binary("userdel").context("userdel binary not found")?;
    run(&userdel, &[username]).with_context(|| format!("delete system user {username}"))?;
    Ok(())
}

/// Recursively change ownership without following symlinks. Entries that
/// vanish mid-walk are skipped (release must stay idempotent).
fn chown_tree(root: &Path, uid: u32, gid: u32) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    chown_one(root, &metadata, uid, gid)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            chown_tree(&entry.path(), uid, gid)?;
        }
    }
    Ok(())
}

fn chown_one(path: &Path, metadata: &std::fs::Metadata, uid: u32, gid: u32) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let target = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let changed = unsafe {
        if metadata.file_type().is_symlink() {
            libc::lchown(target.as_ptr(), uid, gid)
        } else {
            libc::chown(target.as_ptr(), uid, gid)
        }
    };
    ensure!(
        changed == 0,
        "chown {}: {}",
        path.display(),
        std::io::Error::last_os_error()
    );
    Ok(())
}

/// Change only the group of one entry (uid stays untouched), so grants
/// never disturb existing ownership.
pub(super) fn chown_path_gid(path: &Path, metadata: &std::fs::Metadata, gid: u32) -> Result<()> {
    chown_one(path, metadata, u32::MAX, gid)
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("mode {:o} for {}", mode, path.display()))
}

/// Apply the profile ownership to a materialized environment: home and
/// workspace trees (recursive) plus the frozen config snapshot. ReadOnly
/// additionally freezes the trees world-traversable/readable (0555/0444).
pub fn apply_ownership(
    profile: OperatorAccessMode,
    identity: &LinuxIdentity,
    home: &Path,
    workspace: &Path,
    snapshot: &Path,
) -> Result<()> {
    let ownership = tiers::profile_ownership(profile, identity, node_uid(), node_gid());
    let frozen = matches!(profile, OperatorAccessMode::ReadOnly);
    for (path, target) in [(home, ownership.home), (workspace, ownership.workspace)] {
        chown_tree(path, target.uid, target.gid)
            .with_context(|| format!("ownership for {}", path.display()))?;
        if frozen {
            tiers::freeze_read_only_tree(path)
                .with_context(|| format!("freeze {} read-only", path.display()))?;
        }
        set_mode(path, target.mode)?;
    }
    chown_one(
        snapshot,
        &std::fs::symlink_metadata(snapshot)
            .with_context(|| format!("frozen snapshot {} missing", snapshot.display()))?,
        ownership.snapshot.uid,
        ownership.snapshot.gid,
    )?;
    set_mode(snapshot, ownership.snapshot.mode)
}

/// Admission + provisioning + ownership for a fresh operator environment.
/// `Ok(None)` when the config plane disables provisioning (`Off`). The
/// frozen config's `operator.access` tier picks the ownership profile;
/// read-only runs join the shared `opencoder-ro` group and get group-read
/// grants on the node-owned roots they still need.
pub fn materialize_ownership(
    layout: &DirectoryLayout,
    config: &opencoder_core::Config,
    home: &Path,
    workspace: &Path,
    execution_id: &str,
) -> Result<Option<RunAsIdentity>> {
    admit(config)?;
    if matches!(config.operator.linux_identity, LinuxIdentityMode::Off) {
        return Ok(None);
    }
    sweep::sweep_stale(layout);
    let tier = config.operator.access;
    let read_only_gid = match tier {
        OperatorAccessMode::ReadOnly => Some(tiers::ensure_read_only_group_gid()?),
        OperatorAccessMode::ReadWrite => None,
    };
    let memberships = read_only_gid
        .map(|_| vec![tiers::READ_ONLY_GROUP])
        .unwrap_or_default();
    let identity = provision(home, execution_id, &memberships)?;
    if tiers::wants_tmpdir(tier) {
        tiers::prepare_private_tmp(home)?;
    }
    apply_ownership(tier, &identity, home, workspace, &snapshot_path(home))?;
    if let Some(gid) = read_only_gid {
        tiers::grant_group_read(gid, &tiers::read_only_roots(layout, execution_id))?;
    }
    Ok(Some(identity.run_as()))
}

#[cfg(test)]
#[path = "linux_identity/tests.rs"]
mod tests;
