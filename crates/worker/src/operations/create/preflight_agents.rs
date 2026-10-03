/// List agents used by an inline Project executor. Legacy brain modes fail admission.
pub(super) fn project_preflight_agents(todo: &opencoder_store::ProjectTodoRecord) -> Vec<String> {
    use opencoder_store::ProjectExecutorKind;
    match todo.executor_kind {
        ProjectExecutorKind::Agent => vec![todo.agent.clone()],
        ProjectExecutorKind::Team => {
            team_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default()
        }
        ProjectExecutorKind::Dag => {
            dag_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default()
        }
        ProjectExecutorKind::Brain | ProjectExecutorKind::Playbook => Vec::new(),
    }
}

/// Captain + member `node_id`s off an inline team spec; None on a missing or
/// unparseable spec (lazy resolution, never a preflight failure).
fn team_spec_agents(spec: Option<&str>) -> Option<Vec<String>> {
    let spec: opencoder_project::executor::spec::TeamSpec = serde_json::from_str(spec?).ok()?;
    let mut agents = vec![spec.captain.node_id];
    agents.extend(spec.members.into_iter().map(|m| m.node_id));
    Some(agents)
}

/// Agent-step agents off an inline DagSpec (`agent.unwrap_or("act")`); None
/// on a missing or unparseable spec.
fn dag_spec_agents(spec: Option<&str>) -> Option<Vec<String>> {
    let spec: opencoder_dag::DagSpec = serde_json::from_str(spec?).ok()?;
    Some(
        spec.steps
            .into_iter()
            .filter_map(|step| match step.kind.executable() {
                opencoder_dag::StepKind::Agent { agent, .. } => {
                    Some(agent.clone().unwrap_or_else(|| "act".into()))
                }
                _ => None,
            })
            .collect(),
    )
}
