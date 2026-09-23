use super::*;
use opencoder_store::{ProjectExecutorKind, ProjectTodoStatus};

fn todo(kind: ProjectExecutorKind, spec: Option<&str>) -> opencoder_store::ProjectTodoRecord {
    opencoder_store::ProjectTodoRecord {
        id: "pt-1".into(),
        milestone_id: None,
        title: "t".into(),
        draft: "d".into(),
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        agent: "act".into(),
        executor_kind: kind,
        executor_ref: None,
        executor_spec: spec.map(str::to_string),
        active_session_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn preflight_agents_follow_the_executor_kind() {
    // Agent: the todo's own agent.
    assert_eq!(
        project_preflight_agents(&todo(ProjectExecutorKind::Agent, None)),
        vec!["act".to_string()]
    );
    // Team spec: captain + members node_ids; spec-less → lazy skip.
    let team = r#"{"name":"c","captain":{"node_id":"lead","name":"Lead"},"members":[{"node_id":"a1","name":"A"},{"node_id":"a2","name":"B"}]}"#;
    assert_eq!(
        project_preflight_agents(&todo(ProjectExecutorKind::Team, Some(team))),
        vec!["lead".to_string(), "a1".to_string(), "a2".to_string()]
    );
    assert!(project_preflight_agents(&todo(ProjectExecutorKind::Team, None)).is_empty());
    assert!(project_preflight_agents(&todo(ProjectExecutorKind::Team, Some("{"))).is_empty());
    // Dag spec: agent steps with agent.unwrap_or("act"); wasm skipped.
    let dag = r#"{"name":"d","steps":[
        {"name":"w","kind":{"type":"wasm","command":"t.wasm"}},
        {"name":"x","kind":{"type":"agent","prompt":"p","agent":"explore"}},
        {"name":"y","kind":{"type":"agent","prompt":"p"}}]}"#;
    assert_eq!(
        project_preflight_agents(&todo(ProjectExecutorKind::Dag, Some(dag))),
        vec!["explore".to_string(), "act".to_string()]
    );
    assert!(project_preflight_agents(&todo(ProjectExecutorKind::Dag, None)).is_empty());
}

/// Read-only production input fixture, isolated worker storage; no create/dispatch.
#[tokio::test]
#[ignore = "requires retained device-cases admission fixtures on deployment host"]
async fn device_cases_actual_admission_uses_registered_server_profile() {
    use opencoder_core::harness::{CodexSettings, RuntimeSettings, Versioned};
    let pending = "/var/lib/opencoder-device-cases/data/dag/dag-device-cases-single-032-20260923-r2/pending-create.json";
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pending).unwrap()).unwrap();
    let mut assignment: Assignment = serde_json::from_value(value["assignment"].clone()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let workdir = std::path::PathBuf::from("/var/lib/opencoder-device-cases/work");
    let worker = Worker::open(
        crate::WorkerOptions {
            name: "preflight-only".into(),
            workdir: workdir.clone(),
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        None,
    )
    .await
    .unwrap();
    let config = worker.configuration_for(assignment.request.kind).unwrap();
    assert!(!config.agent.runtime.profiles.contains_key("device-cases"));
    let failure = prepare_with_config(&worker, &assignment, false, config.clone(), None);
    assert!(failure
        .err()
        .unwrap()
        .to_string()
        .contains("Codex profile device-cases unavailable"));
    let registered: serde_json::Value = serde_json::from_slice(
        &std::fs::read("/var/tmp/device-cases-release/server-profile.json").unwrap(),
    )
    .unwrap();
    let profile: Versioned<CodexSettings> = serde_json::from_value(registered).unwrap();
    assignment
        .runtime
        .get_or_insert_with(|| Box::new(RuntimeSettings::default()))
        .profiles
        .insert("device-cases".into(), profile);
    let effective = prepare_with_config(&worker, &assignment, false, config, None).unwrap();
    assert!(effective
        .agent
        .runtime
        .profiles
        .contains_key("device-cases"));
    assert!(effective.dag.device_manager.is_some());
    assert!(!root
        .path()
        .join("node/dag")
        .join(&assignment.index.id)
        .join("execution.json")
        .exists());
    assert!(effective
        .agent
        .agents_dir
        .unwrap()
        .join("device-cases/meta.json")
        .is_file());
}
