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

#[cfg(unix)]
#[test]
fn frozen_tool_keeps_its_executable_permissions_and_runs() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("frozen");
    let resource = source.join("tools/probe");
    std::fs::create_dir_all(resource.join("v1")).unwrap();
    std::fs::write(resource.join("meta.json"), r#"{"current":1}"#).unwrap();
    let script = resource.join("v1/probe");
    std::fs::write(&script, "#!/bin/sh\nprintf 'PINNED_TOOL_EXECUTED\\n'\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o750)).unwrap();
    pin(Some(&source), &target).unwrap();
    let pinned = target.join("tools/probe/v1/probe");
    assert_eq!(
        std::fs::metadata(&pinned).unwrap().permissions().mode() & 0o777,
        0o750
    );
    let output = std::process::Command::new(pinned).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"PINNED_TOOL_EXECUTED\n");
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

#[test]
fn catalog_keeps_switchable_identities_and_skips_unreferenced_pools() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("frozen");
    pool(&source, 3);
    // An orphan with invalid metadata and no version must never be opened.
    let orphan = source.join("tools/unused");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("meta.json"), "invalid orphan metadata").unwrap();
    pin_catalog(Some(&source), &target).unwrap();
    for index in 0..3 {
        assert!(target.join(format!("agent-{index}/meta.json")).is_file());
        assert_eq!(
            std::fs::read_to_string(
                target.join(format!("prompts/prompt-{index}/v2/nested/soul.md"))
            )
            .unwrap(),
            format!("version two {index}")
        );
    }
    assert!(!target.join("tools").exists());
    std::fs::write(
        source.join("prompts/prompt-1/v2/nested/soul.md"),
        "new version",
    )
    .unwrap();
    pin_catalog(Some(&source), &target).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("prompts/prompt-1/v2/nested/soul.md")).unwrap(),
        "version two 1"
    );
}

#[test]
fn catalog_rejects_missing_referenced_dependency_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("frozen");
    pool(&source, 3);
    std::fs::remove_dir_all(source.join("prompts/prompt-1")).unwrap();
    assert!(pin_catalog(Some(&source), &target).is_err());
    assert!(!target.exists());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}
