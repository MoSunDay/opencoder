use super::*;
use crate::{runtime::capacity::HostCapacity, DirectoryLayout};
use opencoder_store::fleet::{handoff::RuntimeRecord, FleetStore};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn opening_capacity_never_releases_a_crashed_runtimes_live_slot() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("host.db");
    let store = FleetStore::open(&database).await.unwrap();
    store.initialize_capacity(1).await.unwrap();
    store
        .enqueue_capacity("old-ticket", "old-run", "old")
        .await
        .unwrap();
    assert!(store.claim_capacity("old-ticket", "old").await.unwrap());
    store
        .enqueue_capacity("new-ticket", "new-run", "new")
        .await
        .unwrap();
    std::fs::write(
        root.path().join("host-binding.json"),
        json!({"database":database,"runtime_id":"old"}).to_string(),
    )
    .unwrap();

    let capacity = HostCapacity::load(root.path()).await.unwrap().unwrap();

    assert_eq!(capacity.store.capacity().await.unwrap().running, 1);
    assert!(!store.claim_capacity("new-ticket", "new").await.unwrap());
}

#[tokio::test]
async fn unverified_restart_retains_capacity_instead_of_trusting_an_empty_tracker() {
    let root = tempfile::tempdir().unwrap();
    let data = std::fs::canonicalize(root.path()).unwrap();
    let store = Arc::new(FleetStore::open_memory().await.unwrap());
    store.initialize_capacity(1).await.unwrap();
    store
        .register_runtime(&RuntimeRecord {
            id: "old".into(),
            release_id: "release".into(),
            mode: "staged".into(),
            config: json!({"data_dir":data,"unit":"opencoder-runtime-unverified-test.service"}),
        })
        .await
        .unwrap();
    store
        .enqueue_capacity("ticket", "run", "old")
        .await
        .unwrap();
    assert!(store.claim_capacity("ticket", "old").await.unwrap());
    let capacity = HostCapacity {
        store,
        runtime_id: "old".into(),
    };
    let mut journal = Journal::load(DirectoryLayout::new(data.clone(), None).unwrap()).unwrap();

    assert!(capacity.recover(&data, "node", &mut journal).await.is_err());
    assert_eq!(capacity.store.capacity().await.unwrap().running, 1);
}

#[test]
fn cgroup_ownership_uses_exact_components_and_includes_nested_containers() {
    let unit = "opencoder-runtime-old.service";
    assert!(belongs_to(
        "0::/system.slice/opencoder-runtime-old.service/container",
        unit
    ));
    assert!(!belongs_to(
        "0::/system.slice/opencoder-runtime-old.service-foreign",
        unit
    ));
    assert!(!belongs_to("malformed", unit));
}

#[test]
fn kernel_scan_keeps_old_owners_and_ignores_only_the_current_pid() {
    let root = tempfile::tempdir().unwrap();
    let unit = "opencoder-runtime-old.service";
    for (pid, scope) in [(100, unit), (200, unit), (300, "other.service")] {
        let directory = root.path().join(pid.to_string());
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("cgroup"),
            format!("0::/system.slice/{scope}"),
        )
        .unwrap();
    }
    assert_eq!(kernel_owners(root.path(), unit, 100).unwrap(), vec![200]);
}

#[cfg(target_os = "linux")]
#[test]
fn real_child_is_visible_to_kernel_scan_without_process_tracker_registration() {
    let groups = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let scope = groups
        .lines()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .flat_map(|path| path.split('/'))
        .rfind(|part| !part.is_empty())
        .unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let owners = kernel_owners(Path::new("/proc"), scope, std::process::id());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(owners.unwrap().contains(&child.id()));
}
