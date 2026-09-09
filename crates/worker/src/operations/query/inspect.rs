use super::{pages::message_page_with_budget, view::*};
use crate::{lifecycle::TodoInitialization, Worker};
use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::{json, Value};

pub(in crate::operations) async fn inspect(
    worker: &Worker,
    execution: &ExecutionRef,
) -> Result<RpcReply> {
    if let Some(reply) = super::super::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    let id = execution.id.as_str();
    let snapshot = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(id)
        .map(|record| {
            let request = bounded_request_ref(&record.assignment.request);
            let definition = record
                .assignment
                .definition
                .as_ref()
                .map(|value| bounded_value_ref(value, "definition"));
            let outcome = bounded_value_ref(&record.result, "result");
            let error = record.error.as_deref().map(truncate_error_ref);
            let team_name = record
                .assignment
                .definition
                .as_ref()
                .and_then(|definition| definition["name"].as_str())
                .map(str::to_owned);
            (
                record.annotations.clone(),
                record.assignment.index.clone(),
                record.assignment.request.kind,
                record.assignment.request.target.clone(),
                team_name,
                request,
                definition,
                outcome,
                error,
                record.lifecycle.todo_initialization,
            )
        });
    let Some((
        annotations,
        index,
        kind,
        target,
        team_name,
        request,
        definition,
        outcome,
        error,
        todo_initialization,
    )) = snapshot
    else {
        if let Some(run) = worker
            .inner
            .state
            .project
            .require()?
            .projects
            .get_todo_run_summary(id)
            .await?
        {
            return super::project::inspect(worker, run).await;
        }
        if worker.inner.state.store.get_session(id).await?.is_none() {
            return Ok(RpcReply::error(404, "execution not found"));
        }
        return session_detail(worker, id).await;
    };
    let mut result = json!({"execution":index,"request":request,"definition":definition,"result":outcome,"error":error,"annotations":annotations});
    match kind {
        ExecutionKind::Dag => {
            result["runners"] =
                super::runner::views(worker, &index, result.get("definition")).await?;
        }
        ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator => {
            result["session"] = session_detail(worker, id).await?.body;
        }
        ExecutionKind::Todos => {
            let view = match pre_store_todo_workflow_view(todo_initialization, index.status) {
                Some(view) => view,
                None => classify_todo_workflow(
                    worker.inner.state.store.get_todo_workflow_detail(id).await,
                    todo_initialization,
                    index.status,
                    error.as_deref(),
                )?,
            };
            match view {
                TodoWorkflowView::Ready(workflow) => {
                    let page = worker
                        .inner
                        .state
                        .store
                        .list_todo_items_page(id, None, 100)
                        .await?;
                    result["workflow"] = json!({
                        "workflow": workflow,
                        "items": page.items,
                        "items_page": {
                            "next_ordinal": page.next_ordinal,
                            "more": page.next_ordinal.is_some(),
                        },
                    });
                    result["workflow_initialization"] = json!("ready");
                }
                TodoWorkflowView::Missing {
                    state,
                    initializing,
                } => {
                    result["workflow"] = Value::Null;
                    result["workflow_initialization"] = json!(state);
                    result["workflow_initializing"] = json!(initializing);
                }
                TodoWorkflowView::Inconsistent => {
                    return Ok(RpcReply::error(500, "todo workflow state is inconsistent"));
                }
            }
        }
        ExecutionKind::Team | ExecutionKind::System => {
            let name = if kind == ExecutionKind::System {
                "system"
            } else {
                team_name.as_deref().unwrap()
            };
            let legacy = worker.inner.journal.lock().await.uses_legacy(id);
            let root = if legacy {
                worker.inner.layout.legacy_team_dir(id)?
            } else {
                worker.inner.layout.team_state_dir(kind, id)?
            };
            if opencoder_team::layout::topic_file(&root, name, id)?.exists() {
                result["topic"] = opencoder_team::fs_store::load_topic_summary(&root, name, id)?;
            }
        }
        ExecutionKind::Project => {
            let target = target.as_deref().unwrap();
            let deps = worker.inner.state.project.require()?;
            result["todo"] = deps
                .projects
                .get_todo_summary(target)
                .await?
                .map(|todo| json!(todo))
                .unwrap_or(Value::Null);
            let page = deps.projects.list_todo_runs_page(target, None, 20).await?;
            result["runs"] = json!(page.runs);
            result["runs_page"] = json!({
                "next_version": page.next_version,
                "more": page.next_version.is_some(),
            });
        }
    }
    bounded_reply(result)
}

pub(super) fn pre_store_todo_workflow_view(
    initialization: Option<TodoInitialization>,
    status: ExecutionStatus,
) -> Option<TodoWorkflowView> {
    if initialization != Some(TodoInitialization::Accepted) {
        return None;
    }
    match status {
        ExecutionStatus::Pending | ExecutionStatus::Running => Some(TodoWorkflowView::Missing {
            state: "initializing",
            initializing: true,
        }),
        ExecutionStatus::Cancelling => Some(TodoWorkflowView::Missing {
            state: "stopping",
            initializing: false,
        }),
        _ => None,
    }
}

pub(super) enum TodoWorkflowView {
    Ready(opencoder_store::TodoWorkflowDetail),
    Missing {
        state: &'static str,
        initializing: bool,
    },
    Inconsistent,
}

pub(super) fn classify_todo_workflow(
    lookup: Result<Option<opencoder_store::TodoWorkflowDetail>>,
    initialization: Option<TodoInitialization>,
    status: ExecutionStatus,
    error: Option<&str>,
) -> Result<TodoWorkflowView> {
    if let Some(workflow) = lookup? {
        return Ok(TodoWorkflowView::Ready(workflow));
    }
    if initialization != Some(TodoInitialization::Accepted) {
        return Ok(TodoWorkflowView::Inconsistent);
    }
    let view = match status {
        ExecutionStatus::Pending | ExecutionStatus::Running => TodoWorkflowView::Missing {
            state: "initializing",
            initializing: true,
        },
        ExecutionStatus::Cancelling => TodoWorkflowView::Missing {
            state: "stopping",
            initializing: false,
        },
        ExecutionStatus::Interrupted | ExecutionStatus::Cancelled => TodoWorkflowView::Missing {
            state: "stopped",
            initializing: false,
        },
        ExecutionStatus::Error if error.is_some() => TodoWorkflowView::Missing {
            state: "failed",
            initializing: false,
        },
        ExecutionStatus::Idle | ExecutionStatus::Done | ExecutionStatus::Error => {
            TodoWorkflowView::Inconsistent
        }
    };
    Ok(view)
}

async fn session_detail(worker: &Worker, id: &str) -> Result<RpcReply> {
    let Some(meta) = worker.inner.state.store.get_session(id).await? else {
        return Ok(RpcReply::error(404, "session not found"));
    };
    let page =
        message_page_with_budget(worker, id, MessageCursor::default(), MESSAGE_CHUNK_BYTES).await?;
    let harness = worker
        .inner
        .state
        .store
        .harness_runtime(id)
        .await?
        .unwrap_or_default()
        .harness;
    bounded_reply(json!({"meta":meta,"harness":harness,"messages":page}))
}
