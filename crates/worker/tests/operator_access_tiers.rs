//! Real system round-trips for the operator access tiers: read-write
//! TMPDIR wiring, read-only frozen trees + shared-group grants and the
//! stale-identity sweep. Requires root; skipped elsewhere.
#![cfg(target_os = "linux")]

use opencoder_core::config::{LinuxIdentityMode, OperatorAccessMode, OperatorConfig};
use opencoder_core::fleet::ExecutionKind;
use opencoder_core::Config;
use opencoder_worker::linux_identity as identity;
use opencoder_worker::DirectoryLayout;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

fn root_only() -> bool {
    let running_as_root = unsafe { libc::geteuid() } == 0;
    if !running_as_root {
        eprintln!("skipping: operator access tier tests require root");
    }
    running_as_root
}

fn config(access: OperatorAccessMode) -> Config {
    Config {
        operator: OperatorConfig {
            linux_identity: LinuxIdentityMode::Enforced,
            access,
        },
        ..Default::default()
    }
}

fn layout(root: &Path) -> DirectoryLayout {
    DirectoryLayout::new(root.to_path_buf(), None).unwrap()
}

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

fn owner_of(path: &Path) -> (u32, u32) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    (metadata.uid(), metadata.gid())
}

fn account_exists(username: &str) -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .arg(username)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_as(username: &str, command: &str) -> bool {
    std::process::Command::new("runuser")
        .args(["-u", username, "--", "sh", "-c"])
        .arg(command)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Clean up an account even when an assertion failed mid-test.
struct AccountGuard(String);
impl AccountGuard {
    fn capture(username: String) -> Self {
        Self(username)
    }
}
impl Drop for AccountGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("userdel").arg(&self.0).output();
    }
}

#[test]
fn read_write_tier_sets_the_tmpdir_pair_and_a_private_tmp() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());

    let environment = identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &id,
        &config(OperatorAccessMode::ReadWrite),
    )
    .unwrap()
    .unwrap();
    let run_as_identity = environment.run_as.unwrap();
    let pairs = identity::env_pairs(&environment.home, OperatorAccessMode::ReadWrite);
    let tmpdir = pairs
        .iter()
        .find(|(key, _)| key == "TMPDIR")
        .expect("read-write tier carries a TMPDIR pair");
    assert_eq!(tmpdir.1, environment.home.join("tmp").to_str().unwrap());
    // The read-only tier never gets a scratch directory.
    assert!(
        !identity::env_pairs(&environment.home, OperatorAccessMode::ReadOnly)
            .iter()
            .any(|(key, _)| key == "TMPDIR")
    );
    let tmp = environment.home.join("tmp");
    assert_eq!(mode_of(&tmp), 0o700);
    assert_eq!(owner_of(&tmp), (run_as_identity.uid, run_as_identity.gid));
    drop(guard);
}

#[test]
fn read_only_tier_freezes_trees_and_rejects_operator_writes() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());

    let environment = identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &id,
        &config(OperatorAccessMode::ReadOnly),
    )
    .unwrap()
    .unwrap();
    assert!(environment.run_as.is_some());
    for directory in [&environment.home, &environment.workspace] {
        assert_eq!(
            owner_of(directory),
            (0, 0),
            "{} stays node-owned",
            directory.display()
        );
        assert_eq!(mode_of(directory), 0o555);
    }
    let snapshot = environment.home.join(".opencoder/config.json");
    assert_eq!(mode_of(&snapshot), 0o440);
    // Traversal and reads work; writes inside the frozen trees fail.
    assert!(run_as(
        &username,
        &format!("ls {:?}", environment.workspace)
    ));
    assert!(
        !run_as(
            &username,
            &format!("touch {:?}", environment.workspace.join("x"))
        ),
        "read-only operator must not write its workspace"
    );
    assert!(
        !run_as(
            &username,
            &format!("touch {:?}", environment.home.join("x"))
        ),
        "read-only operator must not write its home"
    );
    drop(guard);
}

#[test]
fn read_only_tier_joins_the_shared_group_and_grants_read_roots() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());

    // Seed the two node-owned roots the session reads: the pinned resource
    // snapshot and the operator skill pool.
    let resources = layout.resources_dir(ExecutionKind::Operator, &id).unwrap();
    std::fs::create_dir_all(resources.join("pack")).unwrap();
    std::fs::write(
        resources.join("pack").join("how.md"),
        "shared read-only body",
    )
    .unwrap();
    let skills = layout.root().join("operator-config").join("skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(skills.join("runbook.md"), "operator runbook").unwrap();

    let environment = identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &id,
        &config(OperatorAccessMode::ReadOnly),
    )
    .unwrap()
    .unwrap();
    let gid = identity::ensure_read_only_group_gid().unwrap();
    let run_as_identity = environment.run_as.unwrap();
    assert!(
        run_as_identity.groups.contains(&gid),
        "read-only identity joins the shared group"
    );
    for granted in [
        resources.join("pack"),
        resources.join("pack").join("how.md"),
        skills.join("runbook.md"),
    ] {
        assert_eq!(
            owner_of(&granted).1,
            gid,
            "{} granted to the group",
            granted.display()
        );
        assert_eq!(
            mode_of(&granted) & 0o040,
            0o040,
            "{} group-readable",
            granted.display()
        );
    }
    assert!(
        run_as(
            &username,
            &format!("cat {:?}", resources.join("pack").join("how.md"))
        ),
        "the operator can read granted roots"
    );
    drop(guard);
}

#[test]
fn read_write_tier_grants_no_shared_group_membership() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());

    let resources = layout.resources_dir(ExecutionKind::Operator, &id).unwrap();
    std::fs::create_dir_all(&resources).unwrap();
    let environment = identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &id,
        &config(OperatorAccessMode::ReadWrite),
    )
    .unwrap()
    .unwrap();
    let run_as_identity = environment.run_as.unwrap();
    let gid = identity::ensure_read_only_group_gid().unwrap();
    assert!(!run_as_identity.groups.contains(&gid));
    assert_ne!(
        owner_of(&resources).1,
        gid,
        "read-write roots keep their group"
    );
    drop(guard);
}

#[test]
fn sweep_releases_directory_less_and_terminal_but_keeps_fresh_accounts() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());

    // Directory gone: the account is an orphan.
    let orphan = format!("operator-{}", ulid::Ulid::new());
    let orphan_name = identity::username_for(&orphan).unwrap();
    let guard = AccountGuard::capture(orphan_name.clone());
    let environment = identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &orphan,
        &config(OperatorAccessMode::ReadWrite),
    )
    .unwrap()
    .unwrap();
    let _ = environment;
    std::fs::remove_dir_all(
        layout
            .execution_dir(ExecutionKind::Operator, &orphan)
            .unwrap(),
    )
    .unwrap();
    identity::sweep_stale(&layout);
    assert!(!account_exists(&orphan_name), "orphaned account swept");

    // Terminal record: the launch-path release was skipped (queue path).
    let terminal = format!("operator-{}", ulid::Ulid::new());
    let terminal_name = identity::username_for(&terminal).unwrap();
    let guard_terminal = AccountGuard::capture(terminal_name.clone());
    identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &terminal,
        &config(OperatorAccessMode::ReadWrite),
    )
    .unwrap()
    .unwrap();
    std::fs::write(
        layout
            .record_path(ExecutionKind::Operator, &terminal)
            .unwrap(),
        r#"{"assignment":{"index":{"status":"done"}}}"#,
    )
    .unwrap();
    identity::sweep_stale(&layout);
    assert!(!account_exists(&terminal_name), "terminal account swept");

    // Fresh creation (directory present, record not yet written): kept.
    let fresh = format!("operator-{}", ulid::Ulid::new());
    let fresh_name = identity::username_for(&fresh).unwrap();
    let guard_fresh = AccountGuard::capture(fresh_name.clone());
    identity::materialize(
        &layout,
        ExecutionKind::Operator,
        &fresh,
        &config(OperatorAccessMode::ReadWrite),
    )
    .unwrap()
    .unwrap();
    identity::sweep_stale(&layout);
    assert!(account_exists(&fresh_name), "in-flight account kept");
    // Idempotent: a second sweep changes nothing.
    identity::sweep_stale(&layout);
    assert!(account_exists(&fresh_name));
    drop(guard);
    drop(guard_terminal);
    drop(guard_fresh);
}
