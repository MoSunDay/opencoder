//! Real system round-trips for per-execution Linux identities: provision
//! (useradd + ownership), materialize/fork (distinct accounts) and release
//! (chown back + userdel). Requires root; skipped elsewhere.
#![cfg(target_os = "linux")]

use opencoder_core::config::{LinuxIdentityMode, OperatorConfig};
use opencoder_core::fleet::ExecutionKind;
use opencoder_core::Config;
use opencoder_worker::linux_identity as identity;
use opencoder_worker::DirectoryLayout;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

fn root_only() -> bool {
    let running_as_root = unsafe { libc::geteuid() } == 0;
    if !running_as_root {
        eprintln!("skipping: operator linux identity tests require root");
    }
    running_as_root
}

fn enforced() -> Config {
    Config::default()
}

fn off() -> Config {
    Config {
        operator: OperatorConfig {
            linux_identity: LinuxIdentityMode::Off,
            ..Default::default()
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
fn provision_creates_the_account_and_materialize_moves_ownership() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());

    let environment = identity::materialize(&layout, ExecutionKind::Operator, &id, &enforced())
        .unwrap()
        .expect("operator materializes");
    let run_as = environment.run_as.expect("identity provisioned");
    assert_eq!(run_as.username.as_deref(), Some(username.as_str()));
    assert!(account_exists(&username));

    let live = identity::lookup(&username).unwrap();
    assert_eq!(live.uid, run_as.uid);
    assert_eq!(live.gid, run_as.gid);
    assert_eq!(live.groups, vec![live.gid]);

    // home/workspace belong to the account (0700), the frozen snapshot
    // stays node-owned 0440.
    for directory in [&environment.home, &environment.workspace] {
        assert_eq!(owner_of(directory), (live.uid, live.gid));
        assert_eq!(mode_of(directory), 0o700);
    }
    let snapshot = environment.home.join(".opencoder/config.json");
    assert_eq!(owner_of(&snapshot), (0, 0));
    assert_eq!(mode_of(&snapshot), 0o440);

    // Idempotent re-materialize reuses the account.
    let again = identity::materialize(&layout, ExecutionKind::Operator, &id, &enforced())
        .unwrap()
        .expect("second materialize");
    assert_eq!(again.run_as.as_ref().unwrap().uid, live.uid);
    drop(guard);
}

#[test]
fn release_hands_files_back_and_deletes_the_account() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());

    let environment = identity::materialize(&layout, ExecutionKind::Operator, &id, &enforced())
        .unwrap()
        .expect("operator materializes");
    let produced = environment.workspace.join("produced.txt");
    // Produce a file AS the execution account (the real writer identity).
    let status = std::process::Command::new("runuser")
        .args(["-u", &username, "--", "sh", "-c"])
        .arg(format!("echo -n operator output > {:?}", produced))
        .status()
        .unwrap();
    assert!(status.success(), "runuser could not write as {username}");
    let account = identity::lookup(&username).unwrap();
    assert_eq!(owner_of(&produced), (account.uid, account.gid));

    identity::release_identity(&layout, ExecutionKind::Operator, &id).unwrap();
    assert!(!account_exists(&username));
    assert_eq!(owner_of(&produced), (0, 0));
    assert_eq!(owner_of(&environment.home), (0, 0));
    // Files survive with their content; only ownership moves.
    assert_eq!(
        std::fs::read_to_string(&produced).unwrap(),
        "operator output"
    );

    // Idempotent: a second release (absent account) succeeds.
    identity::release_identity(&layout, ExecutionKind::Operator, &id).unwrap();
    drop(guard);
}

#[test]
fn fork_provisions_a_fresh_distinct_account() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let parent = format!("operator-{}", ulid::Ulid::new());
    let child = format!("operator-{}", ulid::Ulid::new());
    let version = serde_json::json!(1);

    let parent_env = identity::materialize(&layout, ExecutionKind::Operator, &parent, &enforced())
        .unwrap()
        .expect("parent materializes");
    let child_env = identity::fork_environment(
        &layout,
        ExecutionKind::Operator,
        &parent,
        &child,
        Some(&version),
        &enforced(),
    )
    .unwrap()
    .expect("fork materializes");
    let parent_identity = parent_env.run_as.clone().unwrap();
    let child_identity = child_env.run_as.clone().unwrap();
    let parent_name = identity::username_for(&parent).unwrap();
    let child_name = identity::username_for(&child).unwrap();
    let guards = [
        AccountGuard::capture(parent_name.clone()),
        AccountGuard::capture(child_name.clone()),
    ];
    assert_eq!(
        parent_identity.username.as_deref(),
        Some(parent_name.as_str())
    );
    assert_eq!(
        child_identity.username.as_deref(),
        Some(child_name.as_str())
    );
    assert_ne!(parent_identity.username, child_identity.username);
    assert_ne!(parent_identity.uid, child_identity.uid);
    let child_account = identity::lookup(&child_name).unwrap();
    assert_eq!(child_account.username, child_name);
    let home = layout.home_dir(ExecutionKind::Operator, &child).unwrap();
    assert_eq!(owner_of(&home), (child_account.uid, child_account.gid));
    drop(guards);
}

#[test]
fn off_mode_keeps_the_node_owned_layout_and_admits_any_node() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());

    let environment = identity::materialize(&layout, ExecutionKind::Operator, &id, &off())
        .unwrap()
        .expect("operator materializes");
    assert!(environment.run_as.is_none());
    assert!(!account_exists(&identity::username_for(&id).unwrap()));
    assert_eq!(owner_of(&environment.home), (0, 0));
    let snapshot = environment.home.join(".opencoder/config.json");
    assert_eq!(mode_of(&snapshot), 0o600);
    // Release stays a no-op success without an account.
    identity::release_identity(&layout, ExecutionKind::Operator, &id).unwrap();
}

#[test]
fn identity_annotation_persists_the_provisioned_identity() {
    if !root_only() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let layout = layout(root.path());
    let id = format!("operator-{}", ulid::Ulid::new());
    let username = identity::username_for(&id).unwrap();
    let guard = AccountGuard::capture(username.clone());
    let environment = identity::materialize(&layout, ExecutionKind::Operator, &id, &enforced())
        .unwrap()
        .unwrap();
    let annotation = identity::identity_annotation(&Some(environment));
    assert_eq!(annotation["username"].as_str().unwrap(), username.as_str());
    // Round-trips into the persisted harness runtime field.
    let run_as: opencoder_core::platform::RunAsIdentity =
        serde_json::from_value(annotation).unwrap();
    assert!(run_as.groups.contains(&run_as.gid));
    let disabled = identity::materialize(&layout, ExecutionKind::Operator, &id, &off())
        .unwrap()
        .unwrap();
    assert!(identity::identity_annotation(&Some(disabled)).is_null());
    drop(guard);
}
