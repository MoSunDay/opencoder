use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::types::{ContextMode, TodoSpec, WorkflowSpec, WorkflowState};

/// The same context object is recorded at dispatch and supplied to the child.
pub fn dispatch_context(
    workflow: &WorkflowSpec,
    state: &WorkflowState,
    todo: &TodoSpec,
    mode: ContextMode,
) -> Result<Value> {
    let dependencies = todo
        .depends_on
        .iter()
        .map(|id| {
            let item = state
                .todos
                .get(id)
                .with_context(|| format!("state missing TODO {id}"))?;
            Ok(
                json!({"todo_id":id,"accepted_generation":item.accepted_generation,
            "summary":item.candidate.as_ref().map(|c| &c.summary),
            "result":item.candidate.as_ref().and_then(|c| c.result.as_ref()),
            "verification":item.candidate.as_ref().map(|c| &c.verification),
            "evidence_refs":item.candidate.as_ref().map(|c| &c.evidence_refs)}),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let item = state.todos.get(&todo.id).context("TODO state missing")?;
    let rerun = state.incidents.iter().rev().find(|incident| {
        incident["world_epoch"].as_u64() == Some(state.world_epoch)
            && incident["affected"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &todo.id))
    });
    Ok(json!({
        "workflow_objective":workflow.objective,"constraints":workflow.constraints,
        "todo":todo,"accepted_dependencies":dependencies,"context_mode":mode,
        "previous_recovery":item.candidate.as_ref().map(|c| &c.recovery_context),
        "rerun":rerun.map(|r| json!({"reason":r["reason"],"world_epoch":r["world_epoch"],
            "workspace_policy":"preserve_current_files_and_external_effects"}))
    }))
}

pub fn focused_prompt(context: &Value) -> Result<String> {
    Ok(format!(
        "Complete exactly one focused TODO. You may use available tools and may delegate supporting work through the task tool, but must not advance another TODO. Return only the final Candidate JSON object with fields status(candidate|blocked|interrupted), summary(string), result(string|null), verification(string), evidence_refs(string[]), recovery_context{{summary:string,refs:string[]}}.\n\
         WORKFLOW_OBJECTIVE={}\nCONSTRAINTS={}\nTODO={}\nACCEPTED_DEPENDENCIES={}\nCONTEXT_MODE={}\nPREVIOUS_RECOVERY={}\nRERUN={}",
        context["workflow_objective"],context["constraints"],context["todo"],
        context["accepted_dependencies"],context["context_mode"],context["previous_recovery"],context["rerun"]
    ))
}

/// Scheduling sees results and recovery summaries, never raw child tool logs.
pub fn scheduling_state(state: &WorkflowState) -> Value {
    let todos: serde_json::Map<String, Value> = state
        .todos
        .iter()
        .map(|(id, item)| {
            (
                id.clone(),
                json!({"status":item.status,"attempt":item.attempt,
            "active_session_id":item.active_session_id,"next_context_mode":item.next_context_mode,
            "last_error":item.last_error,"summary":item.candidate.as_ref().map(|c| &c.summary),
            "accepted_generation":item.accepted_generation}),
            )
        })
        .collect();
    json!({"workflow_id":state.workflow_id,"status":state.status,"generation":state.generation,
        "world_epoch":state.world_epoch,"active_todo_ids":state.active_todo_ids,
        "milestones":state.milestones,"todos":todos})
}
