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
