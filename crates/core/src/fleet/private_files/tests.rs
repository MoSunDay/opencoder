use super::*;
use std::os::unix::fs::PermissionsExt;
fn fixture() -> PrivateExecutionContext {
    PrivateExecutionContext {
        expires_at_ms: 1000,
        image_digest: format!("sha256:{}", "a".repeat(64)),
        definition_sha256: "b".repeat(64),
        files: BTreeMap::from([("credential".into(), "fixture-private-grant".into())]),
    }
}
#[test]
fn task_grant_validates_expiry_bounds_paths_and_redacts_debug() {
    let valid = fixture();
    assert!(valid.validate(999).is_ok());
    assert!(valid.validate(1000).is_err());
    assert!(!format!("{valid:?}").contains("fixture-private-grant"));
    let mut invalid = valid.clone();
    invalid.files.insert("../escape".into(), "x".into());
    assert!(invalid.validate(999).is_err());
    invalid = valid;
    invalid.files.insert("large".into(), "x".repeat(512 * 1024));
    assert!(invalid.validate(999).is_err());
}
#[test]
fn materialization_is_private_immutable_and_replayable() {
    let root = tempfile::tempdir().unwrap();
    let value = fixture();
    let path = materialize(root.path(), "dag-example", &value, 1).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(path.join("credential"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        materialize(root.path(), "dag-example", &value, 1).unwrap(),
        path
    );
    let mut changed = value;
    changed.files.insert("credential".into(), "drift".into());
    assert!(materialize(root.path(), "dag-example", &changed, 1).is_err());
    assert_eq!(
        std::fs::read_to_string(path.join("credential")).unwrap(),
        "fixture-private-grant"
    );
}
#[test]
fn private_file_symlinks_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let value = fixture();
    let path = materialize(root.path(), "dag-example", &value, 1).unwrap();
    std::fs::remove_file(path.join("credential")).unwrap();
    std::os::unix::fs::symlink(root.path().join("foreign"), path.join("credential")).unwrap();
    assert!(materialize(root.path(), "dag-example", &value, 1).is_err());
    assert!(!root.path().join("foreign").exists());
}
