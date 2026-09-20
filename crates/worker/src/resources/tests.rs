use super::*;
use serde_json::json;

fn pool(root: &Path, count: usize) {
    for index in 0..count {
        let name = format!("prompt-{index}");
        let resource = root.join("prompts").join(&name);
        std::fs::create_dir_all(resource.join("v2/nested")).unwrap();
        std::fs::create_dir_all(resource.join("v1")).unwrap();
        std::fs::write(resource.join("meta.json"), json!({"current":2}).to_string()).unwrap();
        std::fs::write(
            resource.join("v2/nested/soul.md"),
            format!("version two {index}"),
        )
        .unwrap();
        std::fs::write(resource.join("v1/soul.md"), "old").unwrap();
        let card = root.join(format!("agent-{index}"));
        std::fs::create_dir_all(&card).unwrap();
        std::fs::write(
            card.join("meta.json"),
            json!({"current":{"prompt":name}}).to_string(),
        )
        .unwrap();
    }
}

#[test]
fn copy_version_error_names_source_path() {
    let dest = tempfile::tempdir().unwrap();
    let error = copy::version_files(
        Path::new("/nonexistent-resource-version"),
        &dest.path().join("v1"),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("/nonexistent-resource-version"));
}

#[test]
fn parallel_snapshot_freezes_all_cards_current_versions_and_is_retry_stable() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("frozen");
    pool(&source, 24);
    assert_eq!(pin(Some(&source), &target).unwrap(), Some(target.clone()));
    for index in 0..24 {
        let resource = target.join(format!("prompts/prompt-{index}"));
        assert_eq!(
            std::fs::read_to_string(resource.join("v2/nested/soul.md")).unwrap(),
            format!("version two {index}")
        );
        assert!(!resource.join("v1").exists());
        assert_eq!(
            std::fs::read(target.join(format!("agent-{index}/meta.json"))).unwrap(),
            std::fs::read(source.join(format!("agent-{index}/meta.json"))).unwrap()
        );
    }
    std::fs::write(
        source.join("prompts/prompt-0/v2/nested/soul.md"),
        "changed after acceptance",
    )
    .unwrap();
    pin(Some(&source), &target).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("prompts/prompt-0/v2/nested/soul.md")).unwrap(),
        "version two 0"
    );
}

#[test]
fn failed_parallel_copy_publishes_nothing_and_retry_rebuilds_every_entry() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("frozen");
    pool(&source, 24);
    let broken = source.join("prompts/prompt-12/meta.json");
    std::fs::write(&broken, "{}").unwrap();
    assert!(format!("{:#}", pin(Some(&source), &target).unwrap_err()).contains("no active version"));
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(temp.path()).unwrap().count(),
        1,
        "all worker threads must finish before staging cleanup"
    );
    std::fs::write(broken, json!({"current":2}).to_string()).unwrap();
    pin(Some(&source), &target).unwrap();
    assert_eq!(
        std::fs::read_dir(target.join("prompts")).unwrap().count(),
        24
    );
}

#[test]
fn missing_agent_reference_never_publishes_a_partial_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    pool(&source, 8);
    std::fs::write(
        source.join("agent-0/meta.json"),
        json!({"current":{"prompt":"missing"}}).to_string(),
    )
    .unwrap();
    let target = temp.path().join("frozen");
    assert!(format!("{:#}", pin(Some(&source), &target).unwrap_err()).contains("reference missing"));
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn symlink_in_any_parallel_resource_rejects_the_entire_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    pool(&source, 24);
    std::os::unix::fs::symlink("/etc/passwd", source.join("prompts/prompt-12/v2/escape")).unwrap();
    let target = temp.path().join("frozen");
    assert!(format!("{:#}", pin(Some(&source), &target).unwrap_err()).contains("symlink"));
    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}
