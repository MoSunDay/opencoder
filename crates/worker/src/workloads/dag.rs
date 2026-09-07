use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, Config};
use opencoder_dag::{DagClaimedRun, DagEventBatch, DagStatusReport};
use opencoder_node::uplink::{LocalDagPersistence, Uplink};
use opencoder_store::{EventKind, SessionEventRecord, Store};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct LocalEvents {
    store: Arc<dyn Store>,
    failure: Arc<Mutex<Option<String>>>,
}
#[async_trait::async_trait]
impl LocalDagPersistence for LocalEvents {
    async fn events(&self, batch: &DagEventBatch) -> Result<()> {
        let rows: Vec<_> = batch
            .events
            .iter()
            .map(|e| SessionEventRecord {
                session_id: batch.run_id.clone(),
                kind: EventKind::Step,
                payload: json!({"kind":e.kind,"step":e.step,"payload":e.payload,"at_ms":e.at_ms}),
                ts: e.at_ms,
                seq: None,
                sse_kind: Some(e.kind.clone()),
            })
            .collect();
        match self.store.append_events(&rows).await {
            Ok(_) => Ok(()),
            Err(error) => {
                *self.failure.lock().unwrap() =
                    Some(format!("DAG event persistence failed: {error:#}"));
                Err(error)
            }
        }
    }
    async fn status(&self, _report: &DagStatusReport) -> Result<()> {
        if let Some(error) = self.failure.lock().unwrap().as_ref() {
            anyhow::bail!("{error}");
        }
        Ok(())
    }
}

pub(super) async fn run(
    worker: &Worker,
    record: &Record,
    config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    let assignment = &record.assignment;
    let id = &assignment.index.id;
    let legacy = worker.inner.journal.lock().await.uses_legacy(id);
    let workflow_root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.kind_root(ExecutionKind::Dag)
    };
    let definition = assignment
        .definition
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("DAG definition missing"))?;
    let mut spec: opencoder_dag::DagSpec =
        opencoder_dag::decode_spec(definition.get("spec").unwrap_or(definition))
            .map_err(|e| anyhow::anyhow!(e))?;
    if let Some(input) = assignment.request.input["prompt"]
        .as_str()
        .filter(|p| !p.is_empty())
    {
        for step in &mut spec.steps {
            if let opencoder_dag::StepKind::Agent { prompt, .. } = &mut step.kind {
                *prompt = format!("{prompt}\n执行要求：{input}");
            }
        }
    }
    let input_path = workflow_root.join(id).join("input.json");
    std::fs::create_dir_all(input_path.parent().unwrap())?;
    if !input_path.exists() {
        opencoder_core::atomic_write(&input_path, &serde_json::to_vec(&assignment.request.input)?)?;
    }
    super::agent::create_session(
        worker,
        id,
        "act",
        None,
        Some(spec.name.clone()),
        assignment.index.created_at,
    )
    .await?;
    let failure = Arc::new(Mutex::new(None));
    let uplink = Arc::new(Uplink::for_local_dag(Arc::new(LocalEvents {
        store: worker.inner.state.store.clone(),
        failure: failure.clone(),
    })));
    let deps = opencoder_dag_runtime::RunDeps {
        uplink,
        exec: opencoder_dag_runtime::ExecDeps {
            store: worker.inner.state.store.clone(),
            client: worker.client(&config)?,
            workdir: worker.inner.state.workdir.clone(),
            config,
        },
        workflow_root: workflow_root.clone(),
    };
    let (tx, rx) = tokio::sync::watch::channel(false);
    let fwd = tokio::spawn(async move {
        cancel.cancelled().await;
        let _ = tx.send(true);
    });
    let run = DagClaimedRun {
        run_id: id.clone(),
        dag_id: assignment
            .request
            .target
            .clone()
            .unwrap_or_else(|| spec.name.clone()),
        spec,
        created_at: assignment.index.created_at,
    };
    let status = if resume {
        opencoder_dag_runtime::resume_run(deps, run, rx).await
    } else {
        opencoder_dag_runtime::execute_run(deps, run, rx).await
    };
    fwd.abort();
    if let Some(error) = failure.lock().unwrap().as_ref() {
        anyhow::bail!("{error}");
    }
    let status = status?;
    let mapped = match status {
        opencoder_dag::DagRunStatus::Done => ExecutionStatus::Done,
        opencoder_dag::DagRunStatus::Cancelled => ExecutionStatus::Cancelled,
        _ => ExecutionStatus::Error,
    };
    Ok((
        mapped,
        json!({"run_id":id,"status":status.as_str(),"artifact_root":workflow_root.join(id)}),
    ))
}
