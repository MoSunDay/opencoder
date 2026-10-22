use super::*;

fn root(id: &str, status: ExecutionStatus) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at: 1,
        kind: ExecutionKind::Project,
        node_id: "node".into(),
        status,
    }
}

#[test]
fn project_status_tracks_owned_root_and_unowned_runs_do_not_hide_sessions() {
    let mut cache = InventoryCache::default();
    cache.put_run(ProjectIndexSource {
        id: "agent-run".into(),
        todo_id: "todo".into(),
        created_at: 2,
        status: ProjectTodoRunStatus::Running,
    });
    assert!(cache.index("agent-run", "node").is_none());
    cache.sessions.insert(
        "agent-run".into(),
        SessionIndexSource {
            id: "agent-run".into(),
            kind: Some("agent".into()),
            title: None,
            created_at: 2,
        },
    );
    assert_eq!(
        cache.index("agent-run", "node").unwrap().kind,
        ExecutionKind::Agent
    );
    cache.roots.insert(
        "project-root".into(),
        root("project-root", ExecutionStatus::Pending),
    );
    cache
        .roots_by_todo
        .insert("todo".into(), BTreeSet::from(["project-root".into()]));
    assert_eq!(
        cache.index("agent-run", "node").unwrap().status,
        ExecutionStatus::Pending
    );
    cache.roots.get_mut("project-root").unwrap().status = ExecutionStatus::Running;
    assert_eq!(
        cache.index("agent-run", "node").unwrap().status,
        ExecutionStatus::Interrupted
    );
    cache.active_roots.insert("project-root".into());
    assert_eq!(
        cache.index("agent-run", "node").unwrap().status,
        ExecutionStatus::Running
    );
    cache.runs.get_mut("agent-run").unwrap().status = ProjectTodoRunStatus::Done;
    assert_eq!(
        cache.index("agent-run", "node").unwrap().status,
        ExecutionStatus::Done
    );
    cache.roots.insert(
        "agent-run".into(),
        root("agent-run", ExecutionStatus::Cancelled),
    );
    assert_eq!(
        cache.index("agent-run", "node").unwrap().status,
        ExecutionStatus::Cancelled
    );
}

#[test]
fn replacing_project_membership_does_not_leave_stale_reverse_indexes() {
    let mut cache = InventoryCache::default();
    let mut run = ProjectIndexSource {
        id: "run".into(),
        todo_id: "old".into(),
        created_at: 1,
        status: ProjectTodoRunStatus::Running,
    };
    cache.put_run(run.clone());
    run.todo_id = "new".into();
    run.status = ProjectTodoRunStatus::Done;
    cache.put_run(run);
    assert!(!cache.runs_by_todo.contains_key("old"));
    assert!(cache.running_by_todo.is_empty());
    assert!(cache.runs_by_todo["new"].contains("run"));
    cache.remove_run("run");
    assert!(cache.runs_by_todo.is_empty());
    assert!(cache.runs.is_empty());
}
