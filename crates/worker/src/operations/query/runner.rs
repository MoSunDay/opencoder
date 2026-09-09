use crate::Worker;
use anyhow::Result;
use opencoder_core::fleet::{ExecutionIndex, ExecutionKind};
use serde_json::{json, Value};
use std::path::Path;

async fn optional(path: &Path) -> Result<Value> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn views(
    worker: &Worker,
    index: &ExecutionIndex,
    definition: Option<&Value>,
) -> Result<Value> {
    let Some(steps) = definition
        .and_then(|d| d.get("spec").unwrap_or(d).get("steps"))
        .and_then(Value::as_array)
    else {
        return Ok(json!([]));
    };
    let legacy = worker.inner.journal.lock().await.uses_legacy(&index.id);
    let root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
    };
    let queued = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&index.id)
        .and_then(|record| record.queue.as_ref().map(|queued| queued.config.clone()));
    let mut views = Vec::new();
    for step in steps.iter().filter(|s| s["kind"]["type"] == "runner") {
        let name = step["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Runner step name missing"))?;
        let dir = opencoder_dag::artifacts::step_dir(&root, &index.id, name)
            .map_err(anyhow::Error::msg)?;
        let mut registration = optional(&dir.join("runner-started.json")).await?;
        let started = !registration.is_null();
        if !started {
            if let Some(config) = &queued {
                registration = opencoder_dag_runtime::exec::runner::configuration::snapshot(
                    config,
                    step["kind"]["runner"].as_str().unwrap_or_default(),
                    step["kind"]["agent"].as_str().unwrap_or_default(),
                );
            }
        }
        let status = optional(&dir.join("runner-status.json")).await?;
        let output = optional(&dir.join("output.json")).await?;
        views.push(json!({"step":name,"runner":step["kind"]["runner"],"agent":step["kind"]["agent"],
            "configuration":registration,"started":started,"stage":status["stage"],"detail":status["detail"],
            "summary":output["result"]["summary"],"verdict":output["result"]["verdict"],"artifacts":output["artifacts"]}));
    }
    Ok(json!(views))
}
