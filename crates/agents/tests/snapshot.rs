use opencoder_agents::snapshot::{digest, pin_selected};
use serde_json::json;

#[test]
fn selected_snapshot_pins_only_current_dependencies_and_detects_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("pinned");
    std::fs::create_dir_all(source.join("probe")).unwrap();
    std::fs::create_dir_all(source.join("prompts/prompt/v1")).unwrap();
    std::fs::create_dir_all(source.join("prompts/prompt/v2")).unwrap();
    std::fs::create_dir_all(source.join("unrelated")).unwrap();
    std::fs::write(
        source.join("probe/meta.json"),
        json!({"current":{"prompt":"prompt"}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        source.join("prompts/prompt/meta.json"),
        json!({"current":2}).to_string(),
    )
    .unwrap();
    std::fs::write(source.join("prompts/prompt/v2/how.md"), "original").unwrap();
    pin_selected(Some(&source), &target, Some(&["probe".into()])).unwrap();
    assert!(!target.join("unrelated").exists());
    assert!(!target.join("prompts/prompt/v1").exists());
    let original = digest(&target).unwrap();
    std::fs::remove_dir_all(source).unwrap();
    pin_selected(None, &target, Some(&["probe".into()])).unwrap();
    assert_eq!(digest(&target).unwrap(), original);
    std::fs::write(target.join("prompts/prompt/v2/how.md"), "changed").unwrap();
    assert_ne!(digest(&target).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn fifo_and_symlink_metadata_fail_without_publishing_or_waiting() {
    use std::os::unix::ffi::OsStrExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("probe")).unwrap();
    let meta = source.join("probe/meta.json");
    let name = std::ffi::CString::new(meta.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let target = temp.path().join("pinned");
    assert!(pin_selected(Some(&source), &target, Some(&["probe".into()])).is_err());
    assert!(!target.exists());
    std::fs::remove_file(&meta).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", &meta).unwrap();
    assert!(pin_selected(Some(&source), &target, Some(&["probe".into()])).is_err());
    assert!(!target.exists());
}
