//! Minimal durable sources for node execution inventories.
#[derive(Clone, Debug)]
pub struct SessionIndexSource {
    pub id: String,
    pub kind: Option<String>,
    pub title: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug)]
pub struct ProjectIndexSource {
    pub id: String,
    pub todo_id: String,
    pub created_at: i64,
    pub status: crate::ProjectTodoRunStatus,
}

#[derive(Default, Debug)]
pub struct InventorySources {
    pub revision: u64,
    pub full: bool,
    pub sessions: Vec<SessionIndexSource>,
    pub project_runs: Vec<ProjectIndexSource>,
    pub removed_sessions: Vec<String>,
    pub removed_runs: Vec<String>,
}
