use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

use crate::{domain, types::*};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RerunRequest {
    pub request_id: String,
    pub todo_id: String,
    pub reason: String,
    pub expected_generation: u64,
}

impl RerunRequest {
    pub fn validate(&self) -> Result<()> {
        if !opencoder_core::fleet::valid_id(&self.request_id) {
            bail!("invalid rerun request_id");
        }
        if self.reason.trim().is_empty() || self.reason.len() > 4096 {
            bail!("rerun reason must contain 1..4096 bytes");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RerunPreview {
    pub todo_id: String,
    pub generation: u64,
    pub affected: BTreeSet<String>,
    pub preserved: BTreeSet<String>,
    pub blockers: Vec<String>,
}

pub fn preview(spec: &WorkflowSpec, state: &WorkflowState, todo_id: &str) -> Result<RerunPreview> {
    let todo = spec
        .todos
        .iter()
        .find(|todo| todo.id == todo_id)
        .context("TODO not found")?;
    let mut affected = domain::descendants(spec, todo_id);
    affected.insert(todo_id.to_owned());
    let blockers = todo
        .depends_on
        .iter()
        .filter(|id| {
            !state
                .todos
                .get(*id)
                .is_some_and(|item| item.status == TodoStatus::Passed)
        })
        .cloned()
        .collect();
    Ok(RerunPreview {
        todo_id: todo_id.into(),
        generation: state.generation,
        preserved: state
            .todos
            .keys()
            .filter(|id| !affected.contains(*id))
            .cloned()
            .collect(),
        affected,
        blockers,
    })
}

/// Called only after the owning worker has joined the previous driver.
/// The generation guard belongs to command acceptance, before interruption.
pub fn apply(
    spec: &WorkflowSpec,
    mut state: WorkflowState,
    request: &RerunRequest,
) -> Result<WorkflowState> {
    request.validate()?;
    let preview = preview(spec, &state, &request.todo_id)?;
    if !preview.blockers.is_empty() {
        bail!(
            "rerun prerequisites are not accepted: {}",
            preview.blockers.join(", ")
        );
    }
    if state.status == WorkflowStatus::Running || !state.active_todo_ids.is_empty() {
        bail!("rerun requires a stopped workflow");
    }
    state.world_epoch = state
        .world_epoch
        .checked_add(1)
        .context("workflow epoch exhausted")?;
    for id in &preview.affected {
        let item = state.todos.get_mut(id).context("TODO state missing")?;
        item.status = if id == &request.todo_id {
            TodoStatus::Recovering
        } else {
            TodoStatus::Invalidated
        };
        item.attempt = 0;
        item.accepted_generation = None;
        item.candidate = None;
        item.last_error = Some(request.reason.clone());
        item.next_context_mode = Some(if item.active_session_id.is_some() {
            ContextMode::Fork
        } else {
            ContextMode::New
        });
        state.milestones.remove(id);
    }
    state
        .incidents
        .push(json!({"operation":"rerun","request":request,
        "reason":request.reason,"todo_id":request.todo_id,"affected":preview.affected,
        "world_epoch":state.world_epoch}));
    state.status = WorkflowStatus::Suspended;
    state.terminal_reason = None;
    state.generation = state
        .generation
        .checked_add(1)
        .context("workflow generation exhausted")?;
    Ok(state)
}
