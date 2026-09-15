use crate::{journal::Record, lifecycle::StopIntent, Worker};
use anyhow::{Context, Result};
use opencoder_core::{fleet::*, message::now_ms, Config};
use opencoder_todos::{
    persistence,
    review::rerun::{self, RerunRequest},
    types::*,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Control {
    pub request: RerunRequest,
    pub phase: String,
    pub error: Option<String>,
    pub accepted_at: i64,
    pub generation: Option<u64>,
    pub config: Option<Value>,
}

impl Control {
    pub fn receipt(&self) -> Value {
        json!({"request_id":self.request.request_id,"todo_id":self.request.todo_id,
            "phase":self.phase,"error":self.error,"accepted_at":self.accepted_at,
            "generation":self.generation})
    }
}

pub(super) async fn accept(worker: &Worker, id: &str, input: Value) -> Result<RpcReply> {
    let request: RerunRequest = match serde_json::from_value(input) {
        Ok(request) => request,
        Err(error) => return Ok(RpcReply::error(400, error.to_string())),
    };
    if let Err(error) = request.validate() {
        return Ok(RpcReply::error(400, error.to_string()));
    }
    let _admission = worker.inner.admission.lock().await;
    let gate = worker.lifecycle_gate(id).await;
    let _guard = gate.lock().await;
    let (mut record, legacy) = {
        let journal = worker.inner.journal.lock().await;
        (
            journal
                .records
                .get(id)
                .cloned()
                .context("execution not found")?,
            journal.uses_legacy(id),
        )
    };
    if let Some(control) = record.lifecycle.todo_reruns.get(&request.request_id) {
        return Ok(if control.request != request {
            RpcReply::error(409, "request_id already used with different input")
        } else {
            RpcReply {
                status: 200,
                body: control.receipt(),
            }
        });
    }
    if record
        .lifecycle
        .todo_reruns
        .values()
        .any(|c| c.phase == "stopping")
    {
        return Ok(RpcReply::error(409, "another rerun is in progress"));
    }
    if let Some(error) = worker.admission_error() {
        return Ok(RpcReply::error(503, error));
    }
    if record.assignment.index.status == ExecutionStatus::Cancelling {
        return Ok(RpcReply::error(409, "execution is still stopping"));
    }
    if record.assignment.index.status == ExecutionStatus::Pending {
        return Ok(RpcReply::error(409, "execution is already queued"));
    }
    let Some((spec, state)) = persistence::load(&worker.inner.state.store, id).await? else {
        return Ok(RpcReply::error(409, "workflow is not initialized"));
    };
    if state.generation != request.expected_generation {
        return Ok(RpcReply::error(
            409,
            "workflow changed; refresh the rerun preview",
        ));
    }
    let preview = match rerun::preview(&spec, &state, &request.todo_id) {
        Ok(preview) => preview,
        Err(error) => return Ok(RpcReply::error(404, error.to_string())),
    };
    if !preview.blockers.is_empty() {
        return Ok(RpcReply::error(
            409,
            format!("unaccepted prerequisites: {}", preview.blockers.join(", ")),
        ));
    }
    let config = match super::super::create::prepare(worker, &record.assignment, legacy) {
        Ok(config) => config,
        Err(error) => return Ok(RpcReply::error(400, format!("rerun preflight: {error:#}"))),
    };
    record.lifecycle.stop_intent = None;
    // Durable intent precedes the Store mutation. Recovery checks the same generation.
    record.lifecycle.todo_reruns.insert(
        request.request_id.clone(),
        Control {
            request: request.clone(),
            phase: "stopping".into(),
            error: None,
            accepted_at: now_ms(),
            generation: None,
            config: Some(json!(config)),
        },
    );
    worker.inner.journal.lock().await.save(record.clone())?;
    if let Err(error) = park(worker, &spec, state, &request).await {
        fail(
            worker,
            id,
            &request.request_id,
            format!("rerun acceptance failed: {error:#}"),
        )
        .await?;
        return Ok(RpcReply::error(
            409,
            format!("workflow changed before rerun acceptance: {error:#}"),
        ));
    }
    signal_stop(worker, id).await?;
    Ok(RpcReply {
        status: 202,
        body: record.lifecycle.todo_reruns[&request.request_id].receipt(),
    })
}

async fn park(
    worker: &Worker,
    spec: &WorkflowSpec,
    state: WorkflowState,
    request: &RerunRequest,
) -> Result<()> {
    anyhow::ensure!(
        state.generation == request.expected_generation,
        "workflow generation conflict"
    );
    let previous = json!(state);
    let mut next = opencoder_todos::transitions::reconcile_interrupted(state);
    next.status = WorkflowStatus::Suspended;
    next.terminal_reason = Some(format!("rerun requested: {}", request.reason));
    next.incidents
        .push(json!({"operation":"rerun_requested","request":request}));
    persistence::commit(
        &worker.inner.state.store,
        spec,
        &next,
        "workflow_rerun_requested",
        json!({"request":request,"previous_state":previous}),
    )
    .await?;
    Ok(())
}

async fn signal_stop(worker: &Worker, id: &str) -> Result<()> {
    let active = worker.inner.active.lock().await.get(id).cloned();
    let mut journal = worker.inner.journal.lock().await;
    let mut record = journal
        .records
        .get(id)
        .cloned()
        .context("execution not found")?;
    record.lifecycle.stop_intent = Some(StopIntent::Interrupt);
    record.assignment.index.status = if active.is_some() {
        ExecutionStatus::Cancelling
    } else {
        ExecutionStatus::Interrupted
    };
    journal.save(record)?;
    if let Some(token) = active {
        token.cancel();
    }
    opencoder_session::loop_registry::notify_change();
    Ok(())
}

async fn fail(worker: &Worker, id: &str, request_id: &str, error: String) -> Result<()> {
    let mut journal = worker.inner.journal.lock().await;
    let mut record = journal
        .records
        .get(id)
        .cloned()
        .context("execution not found")?;
    let control = record
        .lifecycle
        .todo_reruns
        .get_mut(request_id)
        .context("rerun receipt missing")?;
    control.phase = "failed".into();
    control.error = Some(error);
    control.config = None;
    journal.save(record)
}

/// Scheduler holds node admission; lifecycle gates exclude driver finalization.
pub(crate) async fn recover_locked(worker: &Worker) -> Result<()> {
    let records: Vec<_> = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .values()
        .filter(|r| {
            r.lifecycle
                .todo_reruns
                .values()
                .any(|c| c.phase == "stopping")
        })
        .cloned()
        .collect();
    for record in records {
        let id = &record.assignment.index.id;
        let gate = worker.lifecycle_gate(id).await;
        let _guard = gate.lock().await;
        let current = worker
            .inner
            .journal
            .lock()
            .await
            .records
            .get(id)
            .cloned()
            .context("execution missing")?;
        let Some(control) = current
            .lifecycle
            .todo_reruns
            .values()
            .find(|c| c.phase == "stopping")
        else {
            continue;
        };
        if let Err(error) = advance(worker, &current, control).await {
            fail(
                worker,
                id,
                &control.request.request_id,
                format!("rerun failed: {error:#}"),
            )
            .await?;
        }
    }
    Ok(())
}

async fn advance(worker: &Worker, record: &Record, control: &Control) -> Result<()> {
    let id = &record.assignment.index.id;
    let request = &control.request;
    let (spec, mut state) = persistence::load(&worker.inner.state.store, id)
        .await?
        .context("workflow missing")?;
    let has = |operation: &str| {
        state
            .incidents
            .iter()
            .any(|i| i["operation"] == operation && i["request"] == json!(request))
    };
    let applied = has("rerun");
    anyhow::ensure!(
        record.lifecycle.stop_intent != Some(StopIntent::Cancel),
        "rerun was cancelled"
    );
    if !has("rerun_requested") {
        park(worker, &spec, state.clone(), request).await?;
        signal_stop(worker, id).await?;
        return Ok(());
    }
    if let Some(token) = worker.inner.active.lock().await.get(id).cloned() {
        token.cancel();
        anyhow::ensure!(
            now_ms() - control.accepted_at <= 30_000,
            "previous execution did not stop within 30 seconds"
        );
        return Ok(());
    }
    if !applied {
        state = rerun::apply(&spec, state, request)?;
        persistence::commit(
            &worker.inner.state.store,
            &spec,
            &state,
            "workflow_rerun_applied",
            json!({"request":request}),
        )
        .await?;
    }
    let config: Config = serde_json::from_value(
        control
            .config
            .clone()
            .context("rerun configuration missing")?,
    )?;
    let mut record = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(id)
        .cloned()
        .context("execution missing")?;
    let receipt = record
        .lifecycle
        .todo_reruns
        .get_mut(&request.request_id)
        .context("rerun receipt missing")?;
    receipt.phase = "queued".into();
    receipt.generation = Some(state.generation);
    receipt.config = None;
    // One durable journal write stores both the receipt and the queue item.
    super::super::queue::enqueue(worker, record, config, true).await?;
    Ok(())
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
