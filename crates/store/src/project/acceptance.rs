//! Preconditions for accepting one observed execution without stale writes.
use crate::{ProjectTodoPatch, ProjectTodoRecord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoAcceptance {
    pub title: String,
    pub draft: String,
    pub initiative_id: Option<String>,
    pub project_id: String,
    pub capability_id: String,
    pub execution_id: String,
    pub required_done: Vec<String>,
    pub all_other_todos: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("TODO acceptance is stale; reread the requirements, dependencies and latest execution")]
pub struct AcceptanceConflict;

impl TodoAcceptance {
    pub fn completed_draft(&self) -> &str {
        self.draft
            .split("\n\n## 当前卡点\n")
            .next()
            .unwrap_or(&self.draft)
    }

    pub fn matches_todo(&self, todo: &ProjectTodoRecord) -> bool {
        todo.title == self.title
            && (todo.draft == self.draft
                || (todo.board_status == "done" && todo.draft == self.completed_draft()))
            && todo.initiative_id == self.initiative_id
            && todo.capability_id.as_deref() == Some(self.capability_id.as_str())
    }

    /// Acceptance may clear the observed blocker, but cannot change its task.
    pub fn permits_patch(&self, patch: &ProjectTodoPatch) -> bool {
        patch.board_status.as_deref() == Some("done")
            && patch.title.as_ref().is_none_or(|v| v == &self.title)
            && patch
                .draft
                .as_deref()
                .is_none_or(|v| v == self.completed_draft())
            && patch
                .initiative_id
                .as_ref()
                .is_none_or(|v| v == &self.initiative_id)
            && patch
                .capability_id
                .as_ref()
                .is_none_or(|v| v.as_ref() == Some(&self.capability_id))
            && patch.agent.is_none()
            && patch.executor_kind.is_none()
            && patch.executor_ref.is_none()
            && patch.executor_spec.is_none()
            && patch.plan_md.is_none()
            && patch.active_session_id.is_none()
            && patch.status.is_none()
    }
}
