//! Shared execution-inventory fixtures (session/todo/run/subagent record
//! builders) reused by the inventory suites.

use opencoder_store::{
    ProjectExecutorKind, ProjectTodoRecord, ProjectTodoRunKind, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus, SessionMeta, SubagentStatus, SubagentTaskRecord,
};

pub(crate) fn session(id: &str, kind: Option<&str>, created_at: i64) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        kind: kind.map(str::to_string),
        created_at,
        updated_at: created_at,
        ..Default::default()
    }
}

pub(crate) fn todo(id: &str, created_at: i64) -> ProjectTodoRecord {
    ProjectTodoRecord {
        people: Default::default(),
        id: id.into(),
        initiative_id: None,
        title: format!("todo {id}"),
        draft: String::new(),
        plan_md: None,
        status: ProjectTodoStatus::Draft,
        board_status: "backlog".into(),
        position: created_at,
        capability_id: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Agent,
        executor_ref: None,
        executor_spec: None,
        active_session_id: None,
        created_at,
        updated_at: created_at,
    }
}

pub(crate) fn run(id: &str, todo_id: &str, created_at: i64) -> ProjectTodoRunRecord {
    ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: id.into(),
        todo_id: todo_id.into(),
        kind: ProjectTodoRunKind::Execute,
        version: 1,
        plan_md: None,
        output_md: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Agent,
        capability_id: None,
        plan_id: None,
        output_ref: None,
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: created_at,
        finished_at: None,
        created_at,
    }
}

pub(crate) fn subagent(child: &str) -> SubagentTaskRecord {
    SubagentTaskRecord {
        task_id: "task".into(),
        parent_session_id: "agent-parent".into(),
        child_session_id: child.into(),
        parent_message_id: None,
        agent: "act".into(),
        prompt: String::new(),
        result: None,
        status: SubagentStatus::Running,
        ok: None,
        started_at: 0,
        completed_at: None,
    }
}
