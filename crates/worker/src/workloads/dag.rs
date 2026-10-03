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
    mut config: Config,
    cancel: CancellationToken,
    resume: bool,
) -> Result<(ExecutionStatus, Value)> {
    let assignment = &record.assignment;
    let id = &assignment.index.id;
    if let Some(private) = &assignment.private_context {
        anyhow::ensure!(
            private.image_digest
                == opencoder_core::fleet::private_files::runtime_image_digest()
                    .map_err(anyhow::Error::msg)?,
            "private task execution image mismatch"
        );
        config.dag.execution_private_root = Some(
            opencoder_core::fleet::private_files::materialize(
                worker.inner.layout.root(),
                id,
                private,
                opencoder_core::message::now_ms(),
            )
            .map_err(anyhow::Error::msg)?,
        );
    }
    let workflow_root = crate::layout::dag::accepted_parent(record)?;
    let definition = assignment
        .definition
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("DAG definition missing"))?;
    let mut spec: opencoder_dag::DagSpec =
        opencoder_dag::decode_spec(definition.get("spec").unwrap_or(definition))
            .map_err(|e| anyhow::anyhow!(e))?;
    apply_input(&mut spec, &assignment.request.input)?;
    let input_path = workflow_root.join(id).join("input.json");
    std::fs::create_dir_all(input_path.parent().unwrap())?;
    if !input_path.exists() {
        opencoder_core::atomic_write(
            &input_path,
            &serde_json::to_vec(execution_input(&assignment.request.input))?,
        )?;
    }
    super::agent::create_session(
        worker,
        id,
        "act",
        None,
        assignment.index.created_at,
        &crate::brain::workdir::node_workdir(worker),
        super::agent::SessionLabels {
            title: Some(spec.name.clone()),
            kind: Some("dag".into()),
        },
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
            .unwrap_or_else(|| assignment.index.id.clone()),
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

/// A registered DAG sees the same named input shape when dispatched directly
/// or through the scheduler. Scheduling metadata remains in its owning record.
fn execution_input(input: &Value) -> &Value {
    if input["brain_layered"].is_object() && input["layered_inputs"].is_object() {
        &input["layered_inputs"]
    } else {
        input
    }
}

fn apply_input(spec: &mut opencoder_dag::DagSpec, input: &Value) -> Result<()> {
    let arguments: Vec<String> = execution_input(input)
        .get("args")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?
        .unwrap_or_default();
    anyhow::ensure!(
        arguments.iter().all(|arg| !arg.contains('\0')),
        "DAG argument contains NUL"
    );
    for step in &mut spec.steps {
        let kind = match &mut step.kind {
            opencoder_dag::StepKind::Dynamic { template, .. } => template.as_mut(),
            kind => kind,
        };
        match kind {
            opencoder_dag::StepKind::Agent { prompt, .. } => {
                if let Some(directive) =
                    input["prompt"].as_str().filter(|prompt| !prompt.is_empty())
                {
                    *prompt = format!("{prompt}\n执行要求：{directive}");
                }
            }
            opencoder_dag::StepKind::Binary { args, .. } => args.extend(arguments.clone()),
            opencoder_dag::StepKind::Dynamic { .. } => {
                unreachable!("nested dynamic kind is invalid")
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directive_and_binary_arguments_keep_exact_values() {
        let mut spec = opencoder_dag::decode_spec(&json!({"name":"test","steps":[
            {"name":"binary","kind":{"type":"binary","resource":"tool","args":["first"]}},
            {"name":"agent","kind":{"type":"agent","prompt":"base"}},
            {"name":"dynamic","kind":{"type":"dynamic","source":{"type":"input","pointer":"/items"},"template":{"type":"binary","resource":"tool","args":[]}}}
        ]})).unwrap();
        apply_input(
            &mut spec,
            &json!({"prompt":"check", "args":["with space", "quoted \"value\"", ""]}),
        )
        .unwrap();
        let opencoder_dag::StepKind::Binary { args, .. } = &spec.steps[0].kind else {
            panic!("binary expected")
        };
        assert_eq!(args, &["first", "with space", "quoted \"value\"", ""]);
        let opencoder_dag::StepKind::Agent { prompt, .. } = &spec.steps[1].kind else {
            panic!("agent expected")
        };
        assert_eq!(prompt, "base\n执行要求：check");
        let opencoder_dag::StepKind::Binary { args, .. } = spec.steps[2].kind.executable() else {
            panic!("binary template expected")
        };
        assert_eq!(args, &["with space", "quoted \"value\"", ""]);
        assert!(apply_input(&mut spec, &json!({"args":"shell string"})).is_err());
        assert!(apply_input(&mut spec, &json!({"args":["nul\u{0000}"]})).is_err());
    }

    #[test]
    fn brain_bound_binary_arguments_reach_the_registered_dag() {
        let mut spec = opencoder_dag::decode_spec(&json!({"name":"bound","steps":[
            {"name":"run","kind":{"type":"binary","resource":"tool","args":["fixed"]}}
        ]}))
        .unwrap();
        apply_input(
            &mut spec,
            &json!({"brain_layered":{"run_id":"brain-bound"},
            "layered_inputs":{"args":["exact code revision", "failed test evidence"]}}),
        )
        .unwrap();
        let opencoder_dag::StepKind::Binary { args, .. } = &spec.steps[0].kind else {
            panic!("binary expected")
        };
        assert_eq!(
            args,
            &["fixed", "exact code revision", "failed test evidence"]
        );
        assert!(apply_input(
            &mut spec,
            &json!({"brain_layered":{},"layered_inputs":{"args":"invalid"}})
        )
        .is_err());
    }

    #[test]
    fn scheduler_named_inputs_preserve_parameters() {
        let payload = json!({"items":["带空格 parameter", "quoted \"value\""]});
        assert_eq!(
            execution_input(&json!({"brain_layered":{"run_id":"root"},"layered_inputs":payload})),
            &payload
        );
        assert_eq!(execution_input(&payload), &payload);
        let ordinary = json!({"layered_inputs":payload});
        assert_eq!(execution_input(&ordinary), &ordinary);
    }
}
