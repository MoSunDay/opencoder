use super::query::native;
use crate::{lifecycle::StopIntent, Worker};
use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::{json, Value};

pub(super) async fn command(
    worker: &Worker,
    execution: &ExecutionRef,
    command: ExecutionCommand,
) -> Result<RpcReply> {
    if let Some(reply) = super::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    let id = execution.id.as_str();
    let record = worker.inner.journal.lock().await.records.get(id).cloned();
    if record.as_ref().is_some_and(|record| {
        record.assignment.request.kind == ExecutionKind::System
            && !matches!(command.action.as_str(), "cancel" | "interrupt")
    }) {
        return Ok(RpcReply::error(
            400,
            "historical system executions only support cancel or interrupt",
        ));
    }
    if record.is_none() && matches!(command.action.as_str(), "cancel" | "interrupt") {
        if let Some(run) = worker
            .inner
            .state
            .project
            .require()?
            .projects
            .get_todo_run(id)
            .await?
        {
            if run.status.is_terminal() {
                return Ok(RpcReply::ok(json!({
                    "id": id,
                    "status": project_run_status(run.status),
                })));
            }
            let (candidates, owners): (usize, Vec<_>) = {
                let journal = worker.inner.journal.lock().await;
                let candidates: Vec<_> = journal
                    .records
                    .values()
                    .filter(|record| {
                        record.assignment.request.kind == ExecutionKind::Project
                            && record.assignment.request.target.as_deref()
                                == Some(run.todo_id.as_str())
                    })
                    .collect();
                let owners = candidates
                    .iter()
                    .filter(|record| record.result["active_run_id"].as_str() == Some(id))
                    .map(|record| record.assignment.index.id.clone())
                    .collect();
                (candidates.len(), owners)
            };
            return match owners.as_slice() {
                [owner] => {
                    let intent = if command.action == "cancel" {
                        StopIntent::Cancel
                    } else {
                        StopIntent::Interrupt
                    };
                    let reply = durable_stop(worker, owner, intent).await?;
                    worker.inner.state.project.cancel(id).await?;
                    Ok(reply)
                }
                [] if candidates == 0 => Ok(RpcReply::ok(
                    json!({"cancelled":worker.inner.state.project.cancel(id).await?}),
                )),
                _ => Ok(RpcReply::error(
                    409,
                    "project run is not the current owned attempt",
                )),
            };
        }
    }
    if command.action == "project-receipt" && execution.kind == ExecutionKind::Project {
        let todo = execution.id.strip_prefix("project-").unwrap_or("");
        let action = command.input["action"].as_str().unwrap_or("");
        return Ok(
            match super::project_admission::existing(worker, todo, action, &command.input["input"])
                .await
            {
                Ok(Some(run)) => super::project_admission::receipt(worker, id, &run).await,
                Ok(None) => RpcReply::error(404, "project run not accepted"),
                Err(error) => RpcReply::error(409, error.to_string()),
            },
        );
    }
    match command.action.as_str() {
        "annotate" => {
            if !command.input.is_object() || serde_json::to_vec(&command.input)?.len() > 64 * 1024 {
                return Ok(RpcReply::error(
                    400,
                    "annotations must be an object of at most 64 KiB",
                ));
            }
            let mut journal = worker.inner.journal.lock().await;
            let Some(mut record) = journal.records.get(id).cloned() else {
                return Ok(RpcReply::error(404, "execution not found"));
            };
            record.annotations = command.input;
            journal.save(record)?;
            Ok(RpcReply::ok(json!({"ok":true})))
        }
        "summary" => match worker.inner.state.store.get_session(id).await? {
            Some(meta) => Ok(RpcReply::ok(json!(meta))),
            None => Ok(RpcReply::error(404, "session not found")),
        },
        "artifact" => super::artifacts::read(worker, id, command.input).await,
        "cancel" | "interrupt" => {
            if record.is_none() {
                return native(
                    worker,
                    "POST",
                    &format!("/api/sessions/{id}/interrupt"),
                    json!({}),
                )
                .await;
            }
            let intent = if command.action == "cancel" {
                StopIntent::Cancel
            } else {
                StopIntent::Interrupt
            };
            durable_stop(worker, id, intent).await
        }
        "resume" | "plan" | "execute" => super::start(worker, id, command).await,
        "prompt" | "steer" | "queue" => {
            let mut body = command.input;
            if command.action != "prompt" {
                body["delivery"] = json!(command.action);
            }
            http(worker, id, "POST", "prompt", body).await
        }
        "http" => {
            let method = command.input["method"].as_str().unwrap_or("GET");
            let tail = command.input["tail"].as_str().unwrap_or("");
            if record.is_some() && method == "POST" && tail_path(tail) == "interrupt" {
                return durable_stop(worker, id, StopIntent::Interrupt).await;
            }
            http(worker, id, method, tail, command.input["body"].clone()).await
        }
        _ => Ok(RpcReply::error(400, "unknown execution command")),
    }
}

fn tail_path(tail: &str) -> &str {
    tail.split_once('?').map_or(tail, |(path, _)| path)
}

fn project_run_status(status: opencoder_store::ProjectTodoRunStatus) -> ExecutionStatus {
    match status {
        opencoder_store::ProjectTodoRunStatus::Running => ExecutionStatus::Running,
        opencoder_store::ProjectTodoRunStatus::Done => ExecutionStatus::Done,
        opencoder_store::ProjectTodoRunStatus::Failed => ExecutionStatus::Error,
        opencoder_store::ProjectTodoRunStatus::Cancelled => ExecutionStatus::Cancelled,
    }
}

pub(crate) async fn durable_stop(
    worker: &Worker,
    id: &str,
    intent: StopIntent,
) -> Result<RpcReply> {
    let gate = worker.lifecycle_gate(id).await;
    let _guard = gate.lock().await;
    let cancel = worker.inner.active.lock().await.get(id).cloned();
    let update = worker
        .inner
        .journal
        .lock()
        .await
        .request_stop(id, intent, cancel.is_some())?;
    if update.signal {
        cancel.expect("active cancellation token").cancel();
    } else {
        let record = worker.inner.journal.lock().await.records.get(id).cloned();
        if let Some(record) = record.filter(|r| r.assignment.index.kind == ExecutionKind::Project) {
            if let Some(run) = record.result["next_run_id"].as_str() {
                worker.inner.state.project.cancel(run).await?;
                worker
                    .inner
                    .state
                    .project
                    .require()?
                    .reserved
                    .lock()
                    .unwrap()
                    .remove(run);
            }
        }
    }
    opencoder_session::loop_registry::notify_change();
    Ok(RpcReply::ok(json!({"id":id,"status":update.status})))
}

async fn http(
    worker: &Worker,
    id: &str,
    method: &str,
    tail: &str,
    body: Value,
) -> Result<RpcReply> {
    if tail.contains("..")
        || tail.starts_with('/')
        || tail.contains('\\')
        || !matches!(method, "GET" | "POST" | "PATCH" | "DELETE")
    {
        return Ok(RpcReply::error(400, "invalid session operation"));
    }
    let _gate = worker.inner.admission.lock().await;
    let (record, legacy) = {
        let journal = worker.inner.journal.lock().await;
        (journal.records.get(id).cloned(), journal.uses_legacy(id))
    };
    if record.as_ref().is_some_and(|r| {
        !matches!(
            r.assignment.request.kind,
            ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator
        )
    }) {
        return Ok(RpcReply::error(
            400,
            "session operation requires a session id",
        ));
    }
    if record
        .as_ref()
        .is_some_and(|r| r.assignment.request.kind == ExecutionKind::Maintenance)
    {
        crate::maintenance_tools::install(worker, id);
    }
    if record.is_none() && method != "GET" && tail != "interrupt" {
        return Ok(RpcReply::error(
            400,
            "managed child sessions are controlled through their owning execution",
        ));
    }
    if requires_admission(method, tail) {
        if let Some(error) = worker.admission_error() {
            return Ok(RpcReply::error(503, error));
        }
    }
    if method == "POST" && tail_path(tail) == "fork" {
        return super::fork::fork(worker, id).await;
    }
    let root = match &record {
        Some(_) if legacy => worker.inner.layout.legacy_resources_dir(id)?,
        Some(record) => worker
            .inner
            .layout
            .resources_dir(record.assignment.index.kind, id)?,
        None => worker
            .inner
            .layout
            .resources_dir(ExecutionKind::Agent, id)?,
    };
    let scope = root.exists().then_some(root);
    let path = format!(
        "/api/sessions/{id}{}{}",
        if tail.is_empty() { "" } else { "/" },
        tail
    );
    let needs_monitor = method == "POST"
        && matches!(tail_path(tail), "prompt" | "compact" | "handoff")
        && !worker.inner.active.lock().await.contains_key(id);
    let permit = if needs_monitor {
        let queued = super::queue::QueuedCommand {
            tail: tail.into(),
            body: body.clone(),
        };
        if let Some(record) = record
            .as_ref()
            .filter(|r| r.assignment.index.status == ExecutionStatus::Pending)
        {
            return Ok(
                if record.queue.as_ref().and_then(|q| q.command.as_ref()) == Some(&queued) {
                    RpcReply::ok(json!({"id":id,"status":"pending"}))
                } else {
                    RpcReply::error(409, "execution already has pending work")
                },
            );
        }
        // FIFO gives earlier pending work newly freed slots. Under LIFO this
        // newest follow-up has priority, just like a freshly enqueued task.
        if worker.inner.scheduling.get().queue_order == QueueOrder::Fifo {
            super::queue::dispatch_locked(worker).await?;
        }
        match worker.try_slot() {
            Some(p) => Some(p),
            None => {
                let Some(mut record) = record.clone() else {
                    return Ok(RpcReply::error(409, "execution owner required"));
                };
                if !crate::lifecycle::can_start(record.assignment.index.status) {
                    return Ok(RpcReply::error(409, "execution is not continuable"));
                }
                let config = super::create::prepare(worker, &record.assignment, legacy)?;
                record.result["monitor_after"] = json!(worker
                    .inner
                    .state
                    .store
                    .events_after(id, 0)
                    .await?
                    .last()
                    .and_then(|e| e.seq)
                    .unwrap_or(0));
                let gate = worker.lifecycle_gate(id).await;
                let _guard = gate.lock().await;
                super::queue::enqueue_with_command(worker, record, config, true, Some(queued))
                    .await?;
                return Ok(RpcReply {
                    status: 202,
                    body: json!({"id":id,"status":"pending"}),
                });
            }
        }
    } else {
        None
    };
    let config = if needs_monitor {
        record
            .as_ref()
            .map(|r| super::create::prepare(worker, &r.assignment, legacy))
            .transpose()?
    } else {
        None
    };
    let mut record = record;
    if needs_monitor {
        if let Some(record) = &mut record {
            let after = worker
                .inner
                .state
                .store
                .events_after(id, 0)
                .await?
                .last()
                .and_then(|e| e.seq)
                .unwrap_or(0);
            record.result["monitor_after"] = json!(after);
        }
    }
    let lifecycle_gate = if needs_monitor && record.is_some() {
        Some(worker.lifecycle_gate(id).await)
    } else {
        None
    };
    if needs_monitor && record.is_some() {
        let status = worker
            .inner
            .journal
            .lock()
            .await
            .records
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("execution not found"))?
            .assignment
            .index
            .status;
        if !crate::lifecycle::can_start(status) {
            return Ok(RpcReply::error(409, "execution is not continuable"));
        }
    }
    let _lifecycle_guard = if let Some(gate) = &lifecycle_gate {
        Some(gate.lock().await)
    } else {
        None
    };
    let result =
        opencoder_core::agent::scope::with_root(scope, native(worker, method, &path, body)).await;
    if result
        .as_ref()
        .is_ok_and(|reply| (200..300).contains(&reply.status))
    {
        if let (Some(permit), Some(record)) = (permit, record) {
            let outcome = super::create::launch_locked(
                worker.clone(),
                record,
                config.expect("monitored execution preflight"),
                permit,
                true,
            )
            .await?;
            if !matches!(outcome, super::create::LaunchOutcome::Started) {
                return Ok(RpcReply::error(
                    409,
                    "execution changed while the session operation was admitted",
                ));
            }
        }
    }
    let reply = result?;
    Ok(reply)
}

fn requires_admission(method: &str, tail: &str) -> bool {
    if method != "POST" {
        return false;
    }
    let path = tail_path(tail);
    matches!(path, "fork" | "prompt" | "compact" | "handoff")
        || path
            .strip_prefix("subagents/")
            .is_some_and(|rest| rest.ends_with("/steer"))
}
