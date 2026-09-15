//! DAG run step query: per-step status/progress derived from the node's
//! on-disk step artifacts (`<workflow_root>/<run>/<step>/meta.json`,
//! `output.json`) and the latest lifecycle event per step. The spec order
//! comes from the journal record's immutable definition snapshot.

use super::view::*;
use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::path::Path;

mod projection;

pub(in crate::operations) async fn dag_steps(
    worker: &Worker,
    execution: &ExecutionRef,
    step: Option<String>,
) -> Result<RpcReply> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(reply);
    }
    if execution.kind != ExecutionKind::Dag {
        return Ok(RpcReply::error(400, "dag steps require a DAG execution"));
    }
    let (status, definition, legacy, execution_error) = {
        let journal = worker.inner.journal.lock().await;
        let record = journal
            .records
            .get(&execution.id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("dag execution not found"))?;
        (
            record.assignment.index.status,
            record.assignment.definition,
            journal.uses_legacy(&execution.id),
            record.error,
        )
    };
    let names = spec_step_names(definition.as_ref());
    if let Some(step) = step.as_deref() {
        if !names.iter().any(|name| name == step) {
            return Ok(RpcReply::error(404, "step not found in run spec"));
        }
    }
    let root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
    };
    let execution_status = status.as_str();
    let snapshot = worker
        .inner
        .state
        .store
        .dag_step_snapshot(&execution.id)
        .await?;
    let events: std::collections::HashMap<_, _> = snapshot
        .steps
        .iter()
        .map(|event| (event.name.as_str(), event))
        .collect();
    match step {
        Some(step) => {
            let meta = step_meta(&root, &execution.id, &step).await?;
            let projected = projection::project(
                &step,
                &meta,
                events.get(step.as_str()).copied(),
                execution_status,
            );
            let output = if meta.is_null()
                || matches!(projected["status"].as_str(), Some("pending" | "running"))
            {
                Value::Null
            } else {
                step_output(&root, &execution.id, &step).await?
            };
            let session_id = step_session_id(&root, &execution.id, &step, &meta).await?;
            let mut body = json!({
                "run_id": execution.id,
                "execution_status": execution_status,
                "name": step,
                "kind": spec_step_kind(definition.as_ref(), &step),
                "status": projected["status"],
                "error": projected["error"],
                "started_at_ms": meta["started_at_ms"],
                "finished_at_ms": meta["finished_at_ms"],
                "output": bounded_value_ref(&output, "output"),
                "head_seq": snapshot.head_seq,
            });
            if let Some(session_id) = session_id {
                body["session_id"] = json!(session_id);
            }
            bounded_reply(body)
        }
        None => {
            let mut rows = Vec::with_capacity(names.len());
            let mut statuses = Vec::with_capacity(names.len());
            for name in &names {
                let meta = step_meta(&root, &execution.id, name).await?;
                rows.push(projection::project(
                    name,
                    &meta,
                    events.get(name.as_str()).copied(),
                    execution_status,
                ));
            }
            statuses.extend(rows.iter().map(|row| row["status"].as_str().unwrap()));
            let (done, error, cancelled, pending) = count_statuses(&statuses);
            bounded_reply(json!({
                "run_id": execution.id,
                "execution_status": execution_status,
                "total": names.len(),
                "done": done,
                "error": error,
                "cancelled": cancelled,
                "pending": pending,
                "running": statuses.iter().filter(|s| **s == "running").count(),
                "interrupted": statuses.iter().filter(|s| **s == "interrupted").count(),
                "head_seq": snapshot.head_seq,
                "execution_error": execution_error.as_deref().map(truncate_error_ref),
                "steps": rows,
            }))
        }
    }
}

/// Project validated metadata; Null means no committed step receipt.
/// Shared with `dag_step_events`, which reports the same projection.
pub(in crate::operations) fn outcome_status(meta: &Value) -> &'static str {
    match meta["outcome"].as_str() {
        Some("done") => "done",
        Some("error") => "error",
        Some("cancelled") => "cancelled",
        _ => "pending",
    }
}

/// Fold statuses into `(done, error, cancelled, pending)` counters. Pure.
fn count_statuses(statuses: &[&str]) -> (usize, usize, usize, usize) {
    statuses.iter().fold((0, 0, 0, 0), |mut counts, status| {
        match *status {
            "done" => counts.0 += 1,
            "error" => counts.1 += 1,
            "cancelled" => counts.2 += 1,
            "pending" => counts.3 += 1,
            _ => {}
        }
        counts
    })
}

/// Only a missing metadata file means the step has not run. Corruption and
/// I/O failures remain visible rather than projecting a false pending state.
/// Shared with `dag_step_events`, which needs the same receipt.
pub(in crate::operations) async fn step_meta(
    root: &Path,
    run_id: &str,
    name: &str,
) -> Result<Value> {
    let dir = opencoder_dag::artifacts::step_dir(root, run_id, name).map_err(anyhow::Error::msg)?;
    match tokio::fs::read(dir.join("meta.json")).await {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                matches!(
                    value["outcome"].as_str(),
                    Some("done" | "error" | "cancelled")
                ),
                "unknown DAG step outcome"
            );
            Ok(value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error.into()),
    }
}
/// The step's child session: `session.json` is committed by the runtime as
/// soon as the session exists, `meta.json.session_id` is the durable
/// fallback. `None` while an agent step has not created its session yet.
/// Shared with `dag_step_events`, which picks its event source with it.
pub(in crate::operations) async fn step_session_id(
    root: &Path,
    run_id: &str,
    name: &str,
    meta: &Value,
) -> Result<Option<String>> {
    let dir = opencoder_dag::artifacts::step_dir(root, run_id, name).map_err(anyhow::Error::msg)?;
    match tokio::fs::read(dir.join("session.json")).await {
        // A torn write is transient: fall back to the committed receipt.
        Ok(bytes) => {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(id) = session_id_field(&value) {
                    return Ok(Some(id));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(session_id_field(meta))
}

/// Accept only a well-formed id; anything else means "not known yet".
fn session_id_field(value: &Value) -> Option<String> {
    value["session_id"]
        .as_str()
        .filter(|id| valid_id(id))
        .map(str::to_owned)
}

async fn step_output(root: &Path, run_id: &str, name: &str) -> Result<Value> {
    let dir = opencoder_dag::artifacts::step_dir(root, run_id, name).map_err(anyhow::Error::msg)?;
    Ok(serde_json::from_slice(
        &tokio::fs::read(dir.join("output.json")).await?,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_status_folds_meta_values() {
        assert_eq!(outcome_status(&json!({"outcome":"done"})), "done");
        assert_eq!(outcome_status(&json!({"outcome":"error"})), "error");
        assert_eq!(outcome_status(&json!({"outcome":"cancelled"})), "cancelled");
        // No meta yet, unreadable shape, or an unknown outcome => pending.
        assert_eq!(outcome_status(&Value::Null), "pending");
        assert_eq!(outcome_status(&json!({"outcome":"weird"})), "pending");
        assert_eq!(outcome_status(&json!({"step":"first"})), "pending");
    }

    #[test]
    fn count_statuses_counts_each_bucket() {
        let metas = [
            json!({"outcome":"done","error":null}),
            json!({"outcome":"error","error":"boom"}),
            json!({"outcome":"cancelled","error":"run cancelled"}),
            json!({"outcome":"done","error":null}),
            Value::Null,
        ];
        let statuses: Vec<&'static str> = metas.iter().map(outcome_status).collect();
        assert_eq!(count_statuses(&statuses), (2, 1, 1, 1));
        assert_eq!(count_statuses(&[]), (0, 0, 0, 0));
    }

    #[test]
    fn spec_step_names_keeps_spec_order() {
        let definition = json!({"spec":{"name":"d","steps":[
            {"name":"first","kind":{"type":"agent","prompt":"a"}},
            {"name":"second","depends_on":["first"],"kind":{"type":"agent","prompt":"b"}}
        ]}});
        assert_eq!(
            spec_step_names(Some(&definition)),
            vec!["first".to_string(), "second".to_string()]
        );
        // Legacy shape: the definition itself carries the steps array.
        let legacy =
            json!({"name":"d","steps":[{"name":"only","kind":{"type":"agent","prompt":"a"}}]});
        assert_eq!(spec_step_names(Some(&legacy)), vec!["only".to_string()]);
        assert!(spec_step_names(None).is_empty());
    }
}
