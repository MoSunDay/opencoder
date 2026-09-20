//! Dynamic instance pages and receipts; all paths derive from validated identities.
use super::{dag_steps, view::bounded_reply};
use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::*;
use opencoder_dag::{dynamic::MAX_INSTANCES, StepKind};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(in crate::operations) struct InstanceContext {
    pub root: PathBuf,
    pub template: StepKind,
    pub items: Option<Vec<Value>>,
    pub execution_status: String,
}

pub(in crate::operations) async fn context(
    worker: &Worker,
    execution: &ExecutionRef,
    step: &str,
) -> Result<Result<InstanceContext, RpcReply>> {
    if let Some(reply) = crate::operations::validate_reference(worker, execution).await? {
        return Ok(Err(reply));
    }
    if execution.kind != ExecutionKind::Dag {
        return Ok(Err(RpcReply::error(
            400,
            "instances require a DAG execution",
        )));
    }
    let (definition, legacy, status) = {
        let journal = worker.inner.journal.lock().await;
        let record = journal.records.get(&execution.id).unwrap();
        (
            record.assignment.definition.clone(),
            journal.uses_legacy(&execution.id),
            record.assignment.index.status.as_str().to_owned(),
        )
    };
    let Some(value) = definition else {
        return Ok(Err(RpcReply::error(404, "DAG definition missing")));
    };
    let spec = opencoder_dag::decode_spec(value.get("spec").unwrap_or(&value))
        .map_err(anyhow::Error::msg)?;
    let Some(node) = spec.steps.iter().find(|s| s.name == step) else {
        return Ok(Err(RpcReply::error(404, "step not found in run spec")));
    };
    let StepKind::Dynamic { template, .. } = &node.kind else {
        return Ok(Err(RpcReply::error(400, "step is not dynamic")));
    };
    let root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
    };
    let items = manifest(&root, &execution.id, step).await?;
    Ok(Ok(InstanceContext {
        root,
        template: template.as_ref().clone(),
        items,
        execution_status: status,
    }))
}

pub(in crate::operations) async fn manifest(
    root: &Path,
    run: &str,
    step: &str,
) -> Result<Option<Vec<Value>>> {
    let dir = opencoder_dag::artifacts::step_dir(root, run, step).map_err(anyhow::Error::msg)?;
    match tokio::fs::read(dir.join("instances.json")).await {
        Ok(bytes) => {
            let items: Vec<Value> = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                items.len() <= MAX_INSTANCES,
                "invalid instance manifest length"
            );
            Ok(Some(items))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(in crate::operations) async fn summary(root: &Path, run: &str, step: &str) -> Result<Value> {
    let dir = opencoder_dag::artifacts::step_dir(root, run, step).map_err(anyhow::Error::msg)?;
    match tokio::fs::read(dir.join("progress.json")).await {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(e.into()),
    }
}

pub(in crate::operations) async fn query(
    worker: &Worker,
    execution: &ExecutionRef,
    step: &str,
    index: Option<usize>,
    offset: usize,
    limit: usize,
) -> Result<RpcReply> {
    let ctx = match context(worker, execution, step).await? {
        Ok(ctx) => ctx,
        Err(reply) => return Ok(reply),
    };
    let items = ctx.items.as_deref().unwrap_or(&[]);
    let progress = summary(&ctx.root, &execution.id, step).await?;
    if let Some(index) = index {
        let Some(input) = items.get(index) else {
            return Ok(RpcReply::error(404, "instance not found"));
        };
        let meta = dag_steps::execution_meta(&ctx.root, &execution.id, step, Some(index)).await?;
        let session =
            dag_steps::execution_session_id(&ctx.root, &execution.id, step, Some(index), &meta)
                .await?;
        let mut result = row(index, &meta, &ctx.execution_status);
        result["run_id"] = json!(execution.id);
        result["name"] = json!(step);
        result["input"] = input.clone();
        result["kind"] = json!(match ctx.template {
            StepKind::Agent { .. } => "agent",
            _ => "wasm",
        });
        result["session_id"] = json!(session);
        result["execution_status"] = json!(ctx.execution_status);
        result["instances"] = progress["instances"].clone();
        if matches!(
            dag_steps::outcome_status(&meta),
            "done" | "error" | "cancelled"
        ) {
            let dir = opencoder_dag::artifacts::execution_dir(
                &ctx.root,
                &execution.id,
                step,
                Some(index),
            )
            .map_err(anyhow::Error::msg)?;
            result["output"] =
                serde_json::from_slice(&tokio::fs::read(dir.join("output.json")).await?)?;
        }
        return bounded_reply(result);
    }
    let limit = limit.clamp(1, 200);
    let mut rows = Vec::new();
    for i in offset..items.len().min(offset.saturating_add(limit)) {
        let meta = dag_steps::execution_meta(&ctx.root, &execution.id, step, Some(i)).await?;
        rows.push(row(i, &meta, &ctx.execution_status));
    }
    bounded_reply(
        json!({"run_id":execution.id,"step":step,"expanded":ctx.items.is_some(),"total":items.len(),
        "offset":offset,"limit":limit,"more":offset.saturating_add(limit)<items.len(),"instances":rows,"progress":progress["instances"]}),
    )
}

pub(in crate::operations) fn status<'a>(meta: &Value, run: &'a str) -> &'a str {
    match dag_steps::outcome_status(meta) {
        "running" | "pending" if matches!(run, "interrupted" | "cancelled" | "error") => run,
        status => status,
    }
}
fn row(index: usize, meta: &Value, run: &str) -> Value {
    json!({"index":index,"status":status(meta,run),"error":meta["error"],
        "started_at_ms":meta["started_at_ms"],"finished_at_ms":meta["finished_at_ms"]})
}
