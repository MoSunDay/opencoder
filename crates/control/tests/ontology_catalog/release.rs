use super::*;
use opencoder_core::{fleet::release::PlatformConfig, ontology::Capability};
use opencoder_ontology::CatalogOperation;
use std::sync::atomic::Ordering;

const SYNC: &str = "/api/ontology/envs/debug/sync";

fn configure(f: &Fixture, id: &str, port: u16) {
    f.state
        .lifecycle
        .platform
        .set(PlatformConfig {
            release_id: id.into(),
            state_dir: f.root.path().into(),
            host_service: "http://127.0.0.1:1".into(),
            resource_service: "http://127.0.0.1:1".into(),
        })
        .unwrap();
    f.state.lifecycle.listener_port.set(port).unwrap();
}

fn journal(f: &Fixture, current: &str, phase: &str, port: u16) {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(f.root.path().join("catalog-writer.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    std::fs::write(
        f.root.path().join("release-state.json"),
        json!({
            "schema_version":1, "current":current, "phase":phase,
            "releases":{current:{"server_port":port}}
        })
        .to_string(),
    )
    .unwrap();
}

async fn seed(f: &Fixture) -> (Value, Value) {
    f.state
        .ontology
        .as_ref()
        .unwrap()
        .catalog(
            "debug".into(),
            CatalogOperation::Apply(vec![Capability {
                key: "platform:current-only".into(),
                source: "platform".into(),
                source_id: "current-only".into(),
                name: "Current capability".into(),
                kind: "platform".into(),
                ..Default::default()
            }]),
        )
        .await
        .unwrap();
    (f.list().await, f.ok("GET", SYNC, Value::Null).await)
}

async fn blocked(f: &Fixture, before: &(Value, Value)) {
    let (status, error) = f.call(Role::Admin, "POST", SYNC, Value::Null).await;
    assert_eq!(status, 502, "{error}");
    assert_eq!(f.list().await, before.0);
    assert_eq!(f.ok("GET", SYNC, Value::Null).await, before.1);
}

async fn accepted(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.state.lifecycle.requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn candidate_startup_and_manual_sync_leave_shared_catalog_and_status_unchanged() {
    let f = Fixture::with_background(false).await;
    let before = seed(&f).await;
    configure(&f, "candidate", 18002);
    journal(&f, "current", "warming", 18001);
    let gate = f
        .state
        .fleet
        .request_lock("ontology_catalog", "debug")
        .await
        .unwrap();
    f.state
        .lifecycle
        .catalog_started
        .store(false, Ordering::SeqCst);
    let _app = opencoder_control::build_app(f.state.clone(), None, false);
    accepted(&f).await;
    drop(gate);
    tokio::time::timeout(Duration::from_secs(5), f.state.lifecycle.drained())
        .await
        .unwrap();
    blocked(&f, &before).await;
    f.ok("GET", &format!("{SYNC}/check"), Value::Null).await;
    // Even a broken candidate source must not write shared Error/Start state.
    std::fs::remove_dir_all(f.root.path().join("agents")).unwrap();
    blocked(&f, &before).await;
}

#[tokio::test]
async fn only_current_activation_can_write_after_switch_or_rollback() {
    let f = Fixture::with_background(false).await;
    let before = seed(&f).await;
    configure(&f, "current", 18001);
    // Missing ownership files, unknown state and standby instances fail closed.
    blocked(&f, &before).await;
    for phase in ["switching", "rolling_back", "migrating", "unknown"] {
        journal(&f, "current", phase, 18001);
        blocked(&f, &before).await;
    }
    journal(&f, "current", "complete", 18003);
    blocked(&f, &before).await;
    journal(&f, "successor", "complete", 18002);
    blocked(&f, &before).await;
    journal(&f, "current", "rolled_back", 18001);
    f.ok("POST", SYNC, Value::Null).await;
    assert_ne!(f.list().await, before.0);
    for phase in [
        "validated",
        "warming",
        "ready",
        "failed",
        "verifying",
        "complete",
    ] {
        journal(&f, "current", phase, 18001);
        f.ok("POST", SYNC, Value::Null).await;
    }
}

#[tokio::test]
async fn queued_old_sync_rechecks_ownership_after_switch() {
    let f = Arc::new(Fixture::with_background(false).await);
    let before = seed(&f).await;
    configure(&f, "current", 18001);
    journal(&f, "current", "complete", 18001);
    let lock = f
        .state
        .fleet
        .request_lock("ontology_catalog", "debug")
        .await
        .unwrap();
    let request = {
        let f = f.clone();
        tokio::spawn(async move { f.call(Role::Admin, "POST", SYNC, Value::Null).await })
    };
    accepted(&f).await;
    journal(&f, "successor", "verifying", 18002);
    drop(lock);
    assert_eq!(request.await.unwrap().0, 502);
    assert_eq!(f.list().await, before.0);
    assert_eq!(f.ok("GET", SYNC, Value::Null).await, before.1);
}
