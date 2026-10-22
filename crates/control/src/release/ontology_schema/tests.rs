use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};

fn fixture() -> (tempfile::TempDir, PlatformConfig, Value) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("server")).unwrap();
    std::fs::create_dir(root.path().join("backup")).unwrap();
    File::create(root.path().join("catalog-writer.lock")).unwrap();
    std::fs::write(root.path().join("backup/manifest.json"), b"verified backup").unwrap();
    let platform = PlatformConfig {
        release_id: "candidate".into(),
        state_dir: root.path().into(),
        host_service: "http://127.0.0.1:1".into(),
        resource_service: "http://127.0.0.1:1".into(),
    };
    let journal = json!({"schema_version":1,"phase":"migrating","current":"old", "candidate":"candidate",
    "releases":{"candidate":{"server_port":18001}},
    "maintenance":{"stage":"installing","target":"candidate","writes_open":false,"schema_started":true,
        "backup":root.path().join("backup"),"ontology_migration":{
            "schema":1,"release_id":"candidate","server_port":18001,
            "database":root.path().join("server/ontology.db"),
            "backup_sha256":format!("{:x}",Sha256::digest(b"verified backup")), "backup_verified":true
        }}});
    (root, platform, journal)
}

fn save(root: &Path, journal: &Value) {
    std::fs::write(root.join("release-state.json"), journal.to_string()).unwrap();
}

#[test]
fn startup_requires_exact_unopened_maintenance_identity_and_backup() {
    let (root, platform, journal) = fixture();
    save(root.path(), &journal);
    let guard = check(&platform, 18001, &root.path().join("server")).unwrap();
    assert_eq!(guard.policy, SchemaPolicy::Maintenance);
    let writer = File::open(root.path().join("catalog-writer.lock")).unwrap();
    assert!(fs2::FileExt::try_lock_exclusive(&writer).is_err());
    drop(guard);
    fs2::FileExt::try_lock_exclusive(&writer).unwrap();
    fs2::FileExt::unlock(&writer).unwrap();
    for pointer in [
        "/maintenance/ontology_migration/release_id",
        "/maintenance/ontology_migration/database",
        "/maintenance/ontology_migration/backup_sha256",
    ] {
        let mut changed = journal.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("mismatched");
        save(root.path(), &changed);
        assert!(check(&platform, 18001, &root.path().join("server")).is_err());
    }
    for (key, value) in [
        ("writes_open", json!(true)),
        ("stage", json!("complete")),
        ("schema_started", json!(false)),
    ] {
        let mut changed = journal.clone();
        changed["maintenance"][key] = value;
        save(root.path(), &changed);
        assert_eq!(
            check(&platform, 18001, &root.path().join("server"))
                .unwrap()
                .policy,
            SchemaPolicy::Validate
        );
    }
    save(root.path(), &journal);
    assert!(check(&platform, 18002, &root.path().join("server")).is_err());
    let mut changed = journal.clone();
    changed["maintenance"]["ontology_migration"]["backup_verified"] = json!(false);
    save(root.path(), &changed);
    assert!(check(&platform, 18001, &root.path().join("server")).is_err());
}

#[test]
fn candidate_prewarm_only_validates_schema_even_with_stale_migration_receipt() {
    let (root, platform, mut journal) = fixture();
    journal["phase"] = json!("warming");
    save(root.path(), &journal);
    assert_eq!(
        check(&platform, 18001, &root.path().join("server"))
            .unwrap()
            .policy,
        SchemaPolicy::Validate
    );
}
