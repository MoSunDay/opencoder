#[path = "../src/resources.rs"]
mod resources;
use serde_json::json;

#[test]
fn absent_configured_source_is_not_an_empty_successful_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("lost-export");
    let target = dir.path().join("node/run/resources");
    let error = resources::pin(Some(&source), &target).unwrap_err();
    assert!(error.to_string().contains("source unavailable"));
    assert!(!target.exists());
    // The built-in-only configuration remains supported explicitly.
    assert_eq!(resources::pin(None, &target).unwrap(), Some(target.clone()));
    // Accepted tasks keep their local snapshot even when the source is gone.
    assert_eq!(
        resources::pin(Some(&source), &target).unwrap(),
        Some(target)
    );
}

#[test]
fn failed_copy_publishes_nothing_and_retry_uses_complete_resources() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("export");
    let target = dir.path().join("node/run/resources");
    resource(&source, "skills", "review", "alpha/SKILL.md", "complete");
    std::fs::write(source.join("skills/review/meta.json"), "broken metadata").unwrap();
    assert!(resources::pin(Some(&source), &target).is_err());
    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
        0
    );
    resource(&source, "skills", "review", "alpha/SKILL.md", "complete");
    resources::pin(Some(&source), &target).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("skills/review/v1/alpha/SKILL.md")).unwrap(),
        "complete"
    );
}

fn resource(source: &std::path::Path, category: &str, name: &str, relative: &str, content: &str) {
    let root = source.join(category).join(name);
    std::fs::create_dir_all(
        root.join("v1").join(
            std::path::Path::new(relative)
                .parent()
                .unwrap_or(std::path::Path::new("")),
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("meta.json"),
        json!({"current":1,"name":name}).to_string(),
    )
    .unwrap();
    std::fs::write(root.join("v1").join(relative), content).unwrap();
}

#[test]
fn pinned_resources_survive_publish_and_resource_removal() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("export");
    let target = dir.path().join("node/resources/run");
    resource(&source, "prompts", "review", "soul.md", "prompt-v1");
    resource(
        &source,
        "skills",
        "review-skills",
        "alpha/SKILL.md",
        "skill-v1",
    );
    resource(&source, "tools", "review-tools", "check", "tool-v1");
    resource(&source, "memory", "review-memory", "memory.md", "memory-v1");
    std::fs::create_dir_all(source.join("reviewer")).unwrap();
    std::fs::write(
        source.join("reviewer/meta.json"),
        json!({"name":"reviewer","current":{
            "prompt":"review",
            "skills":"review-skills",
            "tools":"review-tools",
            "memory":"review-memory"
        }})
        .to_string(),
    )
    .unwrap();
    let pinned = resources::pin(Some(&source), &target).unwrap().unwrap();
    for category in ["prompts", "skills", "tools", "memory", "reviewer"] {
        std::fs::remove_dir_all(source.join(category)).unwrap();
    }
    for (relative, expected) in [
        ("prompts/review/v1/soul.md", "prompt-v1"),
        ("skills/review-skills/v1/alpha/SKILL.md", "skill-v1"),
        ("tools/review-tools/v1/check", "tool-v1"),
        ("memory/review-memory/v1/memory.md", "memory-v1"),
    ] {
        assert_eq!(
            std::fs::read_to_string(pinned.join(relative)).unwrap(),
            expected
        );
    }
    assert!(pinned.join("reviewer/meta.json").is_file());
    assert_eq!(
        resources::pin(Some(&source), &target).unwrap(),
        Some(target)
    );
    assert!(!source.join("executions").exists());
    assert!(resources::check_mount(Some(&source))
        .unwrap_err()
        .to_string()
        .contains("read-only NFS"));
}
#[test]
fn missing_referenced_resources_fail_preflight() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("export");
    std::fs::create_dir_all(source.join("reviewer")).unwrap();
    std::fs::write(
        source.join("reviewer/meta.json"),
        json!({"name":"reviewer","current":{"memory":"missing"}}).to_string(),
    )
    .unwrap();
    assert!(resources::pin(Some(&source), &dir.path().join("run"))
        .unwrap_err()
        .to_string()
        .contains("reference missing"));
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("staging-")));
}

#[cfg(unix)]
#[test]
fn version_root_symlink_cannot_escape_resource_mount() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("export");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(source.join("prompts/review")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret"), "must not be copied").unwrap();
    std::fs::write(
        source.join("prompts/review/meta.json"),
        json!({"current":1,"name":"review"}).to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, source.join("prompts/review/v1")).unwrap();

    let error = resources::pin(Some(&source), &dir.path().join("snapshot"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("version root cannot be a symlink"),
        "{error}"
    );
    assert!(!dir.path().join("snapshot").exists());
}
