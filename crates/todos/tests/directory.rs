use opencoder_todos::{directory, WorkflowSpec};
use serde_json::json;

fn spec() -> WorkflowSpec {
    serde_json::from_value(json!({"schema_version":1,"id":"sample","name":"Sample","objective":"# Goal\n完成任务\n",
        "constraints":["retain context"],"metadata":{"owner":"parent","large":9007199254740993_u64},
        "todos":[{"id":"first","title":"First","requirement_background":"背景\n","instructions":"说明\n",
        "agent":"act","max_attempts":2,"acceptance":{"criteria":"验证","required_tool_calls":[{"name":"bash","arguments_contains":{},"result_ok":false}]},"metadata":{"opaque":[1,true]}}]})).unwrap()
}

#[test]
fn directory_round_trip_preserves_all_spec_fields_and_markdown_bytes() {
    let spec = spec();
    let files = directory::encode(&spec, Some("development")).unwrap();
    let (decoded, binding) = directory::decode(&files).unwrap();
    assert_eq!(decoded, spec);
    assert_eq!(binding.as_deref(), Some("development"));
    assert_eq!(files["objective.md"], "# Goal\n完成任务\n");
    assert!(!files.contains_key("context.json"));
}

#[test]
fn validates_every_required_file_and_returns_file_locations() {
    let mut files = directory::encode(&spec(), None).unwrap();
    files.insert("workflow.json".into(), "{\n  nope\n}".into());
    files.insert("env.json".into(), "{\n  \"env\":\n}".into());
    let errors = directory::validate(&files);
    assert!(errors
        .iter()
        .any(|e| e.path == "workflow.json" && e.line == 2));
    assert!(errors.iter().any(|e| e.path == "env.json" && e.line > 1));
    let mut files = directory::encode(&spec(), None).unwrap();
    files.remove("todos/first/context.md");
    files.insert("todos/first/acceptance.md".into(), "  ".into());
    files.insert("../escape.json".into(), "{}".into());
    let errors = directory::validate(&files);
    assert_eq!(errors.len(), 3);
    assert!(errors.iter().any(|e| e.path == "../escape.json"));
}

#[test]
fn atomic_versions_and_legacy_import_keep_the_source_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("v1");
    std::fs::create_dir(&old).unwrap();
    let original = serde_json::to_string_pretty(&spec()).unwrap();
    std::fs::write(old.join("context.json"), &original).unwrap();
    let mut files = directory::read_files(&old).unwrap();
    files.insert(
        "todos/first/instructions.md".into(),
        "new instructions".into(),
    );
    let next = root.path().join("v2");
    directory::write_new(&next, &files).unwrap();
    assert_eq!(
        directory::load(&next).unwrap().todos[0].instructions,
        "new instructions"
    );
    assert_eq!(
        std::fs::read_to_string(old.join("context.json")).unwrap(),
        original
    );
    assert!(directory::write_new(&next, &files).is_err());
    files.remove("objective.md");
    assert!(directory::write_new(&root.path().join("v3"), &files).is_err());
    assert!(!root.path().join("v3").exists());
}

#[test]
fn unknown_fields_and_invalid_dependencies_fail_without_dropping_data() {
    let mut files = directory::encode(&spec(), None).unwrap();
    let mut task: serde_json::Value =
        serde_json::from_str(&files["todos/first/task.json"]).unwrap();
    task["typo"] = json!(true);
    files.insert("todos/first/task.json".into(), task.to_string());
    assert!(directory::validate(&files)
        .iter()
        .any(|e| e.path == "todos/first/task.json" && e.message.contains("typo")));
    task.as_object_mut().unwrap().remove("typo");
    task["depends_on"] = json!(["missing"]);
    files.insert("todos/first/task.json".into(), task.to_string());
    assert!(directory::validate(&files)
        .iter()
        .any(|e| e.path == "todos/first/task.json" && e.message.contains("missing")));
}

#[cfg(unix)]
#[test]
fn rejects_symlink_files_and_directories() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("todos")).unwrap();
    assert!(directory::read_files(root.path())
        .unwrap_err()
        .to_string()
        .contains("symlinks"));
}

#[test]
fn bound_directory_and_legacy_binding_have_the_same_strict_environment_checks() {
    let root = tempfile::tempdir().unwrap();
    let files = directory::encode(&spec(), Some("dev")).unwrap();
    let path = root.path().join("definition");
    directory::write_new(&path, &files).unwrap();
    assert!(directory::load_bound(&path, root.path()).is_err());
    opencoder_core::share_fs::atomic_write_json(
        &opencoder_core::share_fs::env_context_path(root.path(), "dev").unwrap(),
        &json!({"tools":[],"env_vars":{"TEST_CONTEXT":"pinned"}}),
    )
    .unwrap();
    let frozen = directory::load_bound(&path, root.path()).unwrap();
    assert_eq!(frozen.metadata["env_vars"]["TEST_CONTEXT"], "pinned");
    assert_eq!(frozen.metadata["large"], 9007199254740993_u64);
    let old = root.path().join("legacy");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(
        old.join("context.json"),
        serde_json::to_string(&spec()).unwrap(),
    )
    .unwrap();
    std::fs::write(old.join("env.json"), r#"{"env":123}"#).unwrap();
    let imported = directory::read_files(&old).unwrap();
    assert_eq!(imported["env.json"], r#"{"env":123}"#);
    assert!(directory::decode(&imported).is_err());
}
