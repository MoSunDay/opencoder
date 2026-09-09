//! DAG run step query: per-step status/progress derived from the node's
//! on-disk step artifacts (`<workflow_root>/<run>/<step>/meta.json`,
//! `output.json`). A step with no `meta.json` yet is `pending`; the spec
//! step order comes from the journal record's definition snapshot.

use super::view::*;
use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::path::Path;

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
    let (status, definition, legacy) = {
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
    match step {
        Some(step) => {
            let meta = step_meta(&root, &execution.id, &step).await;
            let output = if outcome_status(&meta) == "pending" {
                Value::Null
            } else {
                step_output(&root, &execution.id, &step).await
            };
            bounded_reply(json!({
                "run_id": execution.id,
                "execution_status": execution_status,
                "name": step,
                "status": outcome_status(&meta),
                "error": meta["error"],
                "started_at_ms": meta["started_at_ms"],
                "finished_at_ms": meta["finished_at_ms"],
                "output": bounded_value_ref(&output, "output"),
            }))
        }
        None => {
            let mut rows = Vec::with_capacity(names.len());
            let mut statuses = Vec::with_capacity(names.len());
            for name in &names {
                let meta = step_meta(&root, &execution.id, name).await;
                statuses.push(outcome_status(&meta));
                rows.push(json!({"name": name, "status": outcome_status(&meta), "error": meta["error"]}));
            }
            let (done, error, cancelled, pending) = count_statuses(&statuses);
            bounded_reply(json!({
                "run_id": execution.id,
                "execution_status": execution_status,
                "total": names.len(),
                "done": done,
                "error": error,
                "cancelled": cancelled,
                "pending": pending,
                "steps": rows,
            }))
        }
    }
}

/// Spec step names in declaration order from the definition snapshot (the
/// same `spec`-aware traversal `runner::views` uses). Pure.
fn spec_step_names(definition: Option<&Value>) -> Vec<String> {
    definition
        .and_then(|d| d.get("spec").unwrap_or(d).get("steps"))
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| step["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Map one step's `meta.json` to its progress status; missing/unreadable/
/// unparseable meta or an unknown outcome folds to `pending`. Pure.
fn outcome_status(meta: &Value) -> &'static str {
    match meta["outcome"].as_str() {
        Some("done") => "done",
        Some("error") => "error",
        Some("cancelled") => "cancelled",
        _ => "pending",
    }
}

/// Fold statuses into `(done, error, cancelled, pending)` counters. Pure.
fn count_statuses(statuses: &[&'static str]) -> (usize, usize, usize, usize) {
    statuses.iter().fold((0, 0, 0, 0), |mut counts, status| {
        match *status {
            "done" => counts.0 += 1,
            "error" => counts.1 += 1,
            "cancelled" => counts.2 += 1,
            _ => counts.3 += 1,
        }
        counts
    })
}

/// Read one step's `meta.json`; IO/parse failures (including a not-yet-run
/// step) yield `Null` so callers see `pending` instead of an error.
async fn step_meta(root: &Path, run_id: &str, name: &str) -> Value {
    let dir = match opencoder_dag::artifacts::step_dir(root, run_id, name) {
        Ok(dir) => dir,
        Err(_) => return Value::Null,
    };
    match tokio::fs::read(dir.join("meta.json")).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

/// Read one step's `output.json` (always written for finished steps;
/// `Value::Null` when absent or unparseable).
async fn step_output(root: &Path, run_id: &str, name: &str) -> Value {
    let dir = match opencoder_dag::artifacts::step_dir(root, run_id, name) {
        Ok(dir) => dir,
        Err(_) => return Value::Null,
    };
    match tokio::fs::read(dir.join("output.json")).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
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
        let legacy = json!({"name":"d","steps":[{"name":"only","kind":{"type":"agent","prompt":"a"}}]});
        assert_eq!(spec_step_names(Some(&legacy)), vec!["only".to_string()]);
        assert!(spec_step_names(None).is_empty());
    }
}
