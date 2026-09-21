use crate::Worker;
use anyhow::{Context, Result};
use opencoder_core::brain::BrainRun;
use opencoder_store::{TodoEventRecord, TodoWorkflowRecord};
use serde_json::{json, Value};

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub async fn load(worker: &Worker, id: &str) -> Result<Option<(TodoWorkflowRecord, BrainRun)>> {
    worker
        .inner
        .state
        .store
        .get_todo_workflow(id)
        .await?
        .map(|record| {
            let run = serde_json::from_value(record.state_json.clone())
                .context("invalid persisted brain state")?;
            Ok((record, run))
        })
        .transpose()
}

/// Caller holds this root's lifecycle gate. Projection, source dedup cursor,
/// wake revision, action ledger and causal event commit in one transaction.
pub async fn save(
    worker: &Worker,
    previous: Option<TodoWorkflowRecord>,
    run: &BrainRun,
    kind: &str,
    payload: Value,
) -> Result<i64> {
    if previous.is_none() {
        crate::workloads::agent::create_session(
            worker,
            &run.id,
            "workflow",
            None,
            run.created_at,
            &crate::brain::workdir::node_workdir(worker),
            crate::workloads::agent::SessionLabels {
                title: Some(run.request.objective.clone()),
                kind: Some("brain".into()),
            },
        )
        .await?;
    }
    let generation = previous.as_ref().map_or(0, |r| r.generation + 1);
    let record = TodoWorkflowRecord {
        id: run.id.clone(),
        parent_session_id: run.id.clone(),
        status: serde_json::to_value(run.phase)?.as_str().unwrap().into(),
        spec_json: previous
            .as_ref()
            .map(|r| r.spec_json.clone())
            .unwrap_or(json!(run.request)),
        state_json: serde_json::to_value(run)?,
        generation,
        created_at: run.created_at,
        updated_at: now(),
        terminal_reason: run.error.clone(),
    };
    let event = TodoEventRecord {
        seq: None,
        workflow_id: run.id.clone(),
        kind: kind.into(),
        payload: json!({"revision":run.revision,"activation":run.activation,"phase":run.phase,"detail":payload,"outputs":run.graph.outputs.values().filter(|o|previous.as_ref().is_none_or(|p|p.state_json["graph"]["outputs"].get(&o.id).is_none())).collect::<Vec<_>>(),"routes":run.graph.routes.values().filter(|r|previous.as_ref().is_none_or(|p|p.state_json["graph"]["routes"].get(&r.context.receipt)!=Some(&json!(r)))).collect::<Vec<_>>()}),
        ts: now(),
    };
    let seq = if previous.is_some() {
        worker
            .inner
            .state
            .store
            .commit_todo_transition(&record, &[], &event)
            .await?
    } else {
        worker
            .inner
            .state
            .store
            .create_todo_workflow(&record, &[], &event)
            .await?
    };
    opencoder_session::loop_registry::notify_change();
    Ok(seq)
}
