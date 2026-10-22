use super::*;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_store::ProjectTodoRunStatus;

impl InventoryCache {
    pub(super) fn index(&self, id: &str, node: &str) -> Option<ExecutionIndex> {
        if let Some(root) = self.roots.get(id) {
            return Some(root.clone());
        }
        if let Some(run) = self.runs.get(id) {
            let root = self.roots_by_todo.get(&run.todo_id).and_then(|ids| {
                ids.iter()
                    .filter_map(|id| self.roots.get(id))
                    .max_by_key(|r| (r.created_at, &r.id))
            });
            if let Some(root) = root {
                let status = match run.status {
                    ProjectTodoRunStatus::Running if root.status == ExecutionStatus::Pending => {
                        ExecutionStatus::Pending
                    }
                    ProjectTodoRunStatus::Running if self.active_roots.contains(&root.id) => {
                        ExecutionStatus::Running
                    }
                    ProjectTodoRunStatus::Running => ExecutionStatus::Interrupted,
                    ProjectTodoRunStatus::Done => ExecutionStatus::Done,
                    ProjectTodoRunStatus::Cancelled => ExecutionStatus::Cancelled,
                    ProjectTodoRunStatus::Failed => ExecutionStatus::Error,
                };
                return Some(ExecutionIndex {
                    id: id.into(),
                    created_at: run.created_at,
                    kind: ExecutionKind::Project,
                    node_id: node.into(),
                    status,
                });
            }
        }
        let session = self.sessions.get(id)?;
        let dag_step = session
            .title
            .as_deref()
            .is_some_and(|title| title.starts_with("dag/"));
        let visible = match session.kind.as_deref() {
            Some("agent" | "team") => true,
            Some("dag") => dag_step,
            Some(_) => false,
            None => {
                id.starts_with("agent-")
                    || dag_step
                    || (id.starts_with("member-")
                        && session.title.as_deref().is_some_and(|t| t.contains(" / ")))
            }
        };
        visible.then(|| ExecutionIndex {
            id: id.into(),
            created_at: session.created_at,
            kind: ExecutionKind::Agent,
            node_id: node.into(),
            status: if self.active_loops.contains(id) {
                ExecutionStatus::Running
            } else {
                ExecutionStatus::Done
            },
        })
    }

    pub(super) fn remove_run(&mut self, id: &str) {
        if let Some(old) = self.runs.remove(id) {
            remove_member(&mut self.runs_by_todo, &old.todo_id, id);
            remove_member(&mut self.running_by_todo, &old.todo_id, id);
        }
    }
    pub(super) fn put_run(&mut self, run: ProjectIndexSource) {
        self.remove_run(&run.id);
        self.runs_by_todo
            .entry(run.todo_id.clone())
            .or_default()
            .insert(run.id.clone());
        if run.status == ProjectTodoRunStatus::Running {
            self.running_by_todo
                .entry(run.todo_id.clone())
                .or_default()
                .insert(run.id.clone());
        }
        self.runs.insert(run.id.clone(), run);
    }
}

#[cfg(test)]
mod tests;
pub(super) fn remove_member(map: &mut BTreeMap<String, BTreeSet<String>>, key: &str, id: &str) {
    if let Some(ids) = map.get_mut(key) {
        ids.remove(id);
        if ids.is_empty() {
            map.remove(key);
        }
    }
}
