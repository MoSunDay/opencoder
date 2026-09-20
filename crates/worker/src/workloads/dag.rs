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
    apply_input(&mut spec, &assignment.request.input);
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
        &crate::brain::workdir::node_workdir(worker),
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
            workdir: crate::brain::workdir::for_record(worker, record)?,
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

/// Fold the dispatch input into the frozen spec before execution (pure —
/// unit-tested here): `prompt` appends an execution directive to every
/// agent step, and `args` (non-empty string) appends to every wasm step's
/// command line. The wasm executor's `split_command` whitespace split
/// makes the join transparent, and every run/resume decodes the frozen
/// definition fresh, so the rewrite never accumulates across retries.
fn apply_input(spec: &mut opencoder_dag::DagSpec, input: &Value) {
    if let Some(directive) = input["prompt"].as_str().filter(|p| !p.is_empty()) {
        for step in &mut spec.steps {
            if let opencoder_dag::StepKind::Agent { prompt, .. } = &mut step.kind {
                *prompt = format!("{prompt}\n执行要求：{directive}");
            }
        }
    }
    if let Some(args) = input["args"].as_str().filter(|a| !a.trim().is_empty()) {
        for step in &mut spec.steps {
            if let opencoder_dag::StepKind::Wasm { command, .. } = &mut step.kind {
                *command = format!("{command} {args}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> opencoder_dag::DagSpec {
        opencoder_dag::decode_spec(&serde_json::json!({
            "name": "t",
            "steps": [
                {"name": "w", "kind": {"type": "wasm", "command": "tool.wasm"}},
                {"name": "a", "kind": {"type": "agent", "prompt": "base"}},
            ],
        }))
        .expect("fixture spec")
    }

    fn wasm_command(spec: &opencoder_dag::DagSpec) -> &str {
        match &spec.steps[0].kind {
            opencoder_dag::StepKind::Wasm { command, .. } => command,
            _ => panic!("step 0 must be wasm"),
        }
    }

    fn agent_prompt(spec: &opencoder_dag::DagSpec) -> &str {
        match &spec.steps[1].kind {
            opencoder_dag::StepKind::Agent { prompt, .. } => prompt,
            _ => panic!("step 1 must be agent"),
        }
    }

    #[test]
    fn args_append_to_every_wasm_command_only() {
        let mut spec = spec();
        apply_input(&mut spec, &serde_json::json!({"args": "--date 2026-09-18"}));
        assert_eq!(wasm_command(&spec), "tool.wasm --date 2026-09-18");
        assert_eq!(agent_prompt(&spec), "base");
    }

    #[test]
    fn prompt_appends_the_execution_directive_to_agent_steps_only() {
        let mut spec = spec();
        apply_input(&mut spec, &serde_json::json!({"prompt": "聚焦告警"}));
        assert_eq!(agent_prompt(&spec), "base\n执行要求：聚焦告警");
        assert_eq!(wasm_command(&spec), "tool.wasm");
    }

    #[test]
    fn empty_or_whitespace_values_are_no_ops() {
        for input in [
            serde_json::json!({}),
            serde_json::json!({"args": "", "prompt": ""}),
            serde_json::json!({"args": "   "}),
        ] {
            let mut spec = spec();
            apply_input(&mut spec, &input);
            assert_eq!(wasm_command(&spec), "tool.wasm", "{input}");
            assert_eq!(agent_prompt(&spec), "base", "{input}");
        }
    }

    #[test]
    fn prompt_and_args_coexist() {
        let mut spec = spec();
        apply_input(
            &mut spec,
            &serde_json::json!({"prompt": "巡检", "args": "--mode strict"}),
        );
        assert_eq!(wasm_command(&spec), "tool.wasm --mode strict");
        assert_eq!(agent_prompt(&spec), "base\n执行要求：巡检");
    }
}
