//! Shared four-slot scheduler. Round-robin logical nodes, per-instance cancellation.
use super::{
    dynamic::{self, Group},
    StepDone, MAX_CONCURRENT_STEPS,
};
use crate::{
    dag_events::{step_started_event, RunEventSink},
    exec::{ExecDeps, StepCtx, StepResult},
    step_io::{mark_unfinished, record_step},
};
use anyhow::Result;
use opencoder_core::message::now_ms;
use opencoder_dag::{DagClaimedRun, StepKind, StepOutcome, StepOutputs, StepStates};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
use tokio::{sync::watch, task::JoinSet};
use tokio_util::sync::CancellationToken;

type Key = (String, Option<usize>);

pub(super) async fn schedule(
    exec: Arc<ExecDeps>,
    root: &Path,
    run: &DagClaimedRun,
    sink: &RunEventSink,
    mut cancel_rx: watch::Receiver<bool>,
    resume: bool,
) -> Result<(StepStates, BTreeMap<String, String>, bool)> {
    let mut states = StepStates::new();
    let mut outputs = StepOutputs::new();
    let mut errors = BTreeMap::new();
    if resume {
        crate::checkpoint::restore(root, run, &mut states, &mut outputs)?;
    }
    let input_path = root.join(&run.run_id).join("input.json");
    let input: Value = match std::fs::read(input_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(e) => return Err(e.into()),
    };
    for step in &run.spec.steps {
        if !states.contains_key(&step.name) {
            opencoder_core::agent::scope::with_root_sync(
                exec.config.agent.agents_dir.clone(),
                || crate::exec::how_copy::freeze(root, &run.run_id, step),
            )?;
        }
    }
    let cancel = CancellationToken::new();
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut active: BTreeSet<Key> = BTreeSet::new();
    let mut tasks: JoinSet<StepDone> = JoinSet::new();
    let mut cursor = 0usize;
    let result: Result<()> = async { loop {
        if *cancel_rx.borrow_and_update() {
            cancel.cancel();
        }
        if cancel.is_cancelled() {
            for (name, group) in &mut groups {
                group.token.cancel();
                group.cancel_pending(root, run, name).await?;
                group.progress(root, &run.run_id, name, sink)?;
            }
        }
        // One pass across nodes per slot. Advancing the cursor prevents a large
        // expansion from monopolizing slots while another branch is ready.
        if !cancel.is_cancelled() {
            let mut misses = 0;
            while tasks.len() < MAX_CONCURRENT_STEPS && misses < run.spec.steps.len() {
                let step = &run.spec.steps[cursor];
                cursor = (cursor + 1) % run.spec.steps.len();
                misses += 1;
                if states.contains_key(&step.name)
                    || !step
                        .depends_on
                        .iter()
                        .all(|n| states.get(n) == Some(&StepOutcome::Done))
                {
                    continue;
                }
                let (instance, instance_input, kind, token) = match &step.kind {
                    StepKind::Dynamic { template, .. } => {
                        if !groups.contains_key(&step.name) {
                            match dynamic::open(
                                root,
                                run,
                                step,
                                &input,
                                &outputs,
                                cancel.child_token(),
                                resume,
                            ) {
                                Ok(group) => {
                                    dynamic::start(
                                        root,
                                        &run.run_id,
                                        &step.name,
                                        None,
                                        group.started,
                                    )?;
                                    sink.emit(step_started_event(&step.name));
                                    group.progress(root, &run.run_id, &step.name, sink)?;
                                    groups.insert(step.name.clone(), group);
                                }
                                Err(e) => {
                                    record_step(
                                        StepDone {
                                            name: step.name.clone(),
                                            instance: None,
                                            started_at_ms: now_ms(),
                                            result: failed(format!("dynamic expansion: {e:#}")),
                                        },
                                        &mut states,
                                        &mut outputs,
                                        &mut errors,
                                        root,
                                        run,
                                        sink,
                                    )
                                    .await;
                                    misses = 0;
                                    continue;
                                }
                            }
                        }
                        let group = groups.get_mut(&step.name).unwrap();
                        if let Some(result) = group.result() {
                            record_step(
                                StepDone {
                                    name: step.name.clone(),
                                    instance: None,
                                    started_at_ms: group.started,
                                    result,
                                },
                                &mut states,
                                &mut outputs,
                                &mut errors,
                                root,
                                run,
                                sink,
                            )
                            .await;
                            misses = 0;
                            continue;
                        }
                        let Some(i) = group.next() else {
                            continue;
                        };
                        group.running.insert(i);
                        (
                            Some(i),
                            Some(group.items[i].clone()),
                            template.as_ref().clone(),
                            group.token.child_token(),
                        )
                    }
                    kind => {
                        if active.contains(&(step.name.clone(), None)) {
                            continue;
                        }
                        (None, None, kind.clone(), cancel.child_token())
                    }
                };
                misses = 0;
                let at = now_ms();
                dynamic::start(root, &run.run_id, &step.name, instance, at)?;
                let mut event = step_started_event(&step.name);
                if let Some(i) = instance {
                    event.kind = "instance_started".into();
                    event.payload = json!({"index":i});
                }
                sink.emit(event);
                if let Some(group) = groups.get(&step.name) {
                    group.progress(root, &run.run_id, &step.name, sink)?;
                }
                let mut executable = step.clone();
                executable.kind = kind;
                let ctx = StepCtx {
                    run_id: run.run_id.clone(),
                    instance,
                    instance_input,
                    spec: run.spec.clone(),
                    step: executable,
                    states: states.clone(),
                    outputs: outputs.clone(),
                    workflow_root: root.into(),
                    log: Some(sink.step_log(&step.name).with_instance(instance)),
                    knowledge_root: exec.config.dag.knowledge_root.clone(),
                    ops: exec.config.dag.ops.clone(),
                };
                active.insert((step.name.clone(), instance));
                let exec = exec.clone();
                tasks.spawn(async move {
                    let result = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
                        super::execute_step(&ctx, &exec, token),
                    ))
                    .await
                    .unwrap_or_else(|_| failed("step executor panicked".into()));
                    StepDone {
                        name: ctx.step.name,
                        instance,
                        started_at_ms: at,
                        result,
                    }
                });
            }
        }
        // Fold fully drained groups, including failure-triggered sibling cancellation.
        for (name, group) in &groups {
            if states.contains_key(name) {
                continue;
            }
            if let Some(result) = group.result() {
                record_step(
                    StepDone {
                        name: name.clone(),
                        instance: None,
                        started_at_ms: group.started,
                        result,
                    },
                    &mut states,
                    &mut outputs,
                    &mut errors,
                    root,
                    run,
                    sink,
                )
                .await;
            }
        }
        if tasks.is_empty() {
            if !cancel.is_cancelled() && !opencoder_dag::ready_steps(&run.spec, &states).is_empty()
            {
                continue;
            }
            mark_unfinished(
                &run.spec,
                &mut states,
                &mut errors,
                root,
                run,
                sink,
                if cancel.is_cancelled() {
                    "run cancelled"
                } else {
                    "blocked: upstream step did not succeed"
                },
                if cancel.is_cancelled() {
                    StepOutcome::Cancelled
                } else {
                    StepOutcome::Error
                },
            )
            .await;
            break;
        }
        tokio::select! {
            biased;
            _ = super::await_flag(&mut cancel_rx), if !cancel.is_cancelled() => { cancel.cancel(); }
            done = tasks.join_next() => {
                let Some(done) = done else { continue; };
                let mut done = done?;
                active.remove(&(done.name.clone(), done.instance));
                if let Some(i) = done.instance {
                    if let Err(e) = crate::step_io::write_execution_artifacts(root, &run.run_id, &done.name, Some(i), done.started_at_ms, &done.result).await {
                        done.result = failed(format!("instance artifact persistence: {e:#}"));
                    }
                    let group = groups.get_mut(&done.name).unwrap();
                    group.running.remove(&i);
                    group.outcomes[i] = Some(done.result.outcome);
                    group.outputs[i] = done.result.output_json.clone().unwrap_or(Value::Null);
                    sink.emit(opencoder_dag::DagEventIn { kind:"instance_done".into(), step:Some(done.name.clone()),
                        payload:json!({"index":i,"ok":done.result.outcome.is_success(),"outcome":done.result.outcome,"error":done.result.error}), at_ms:now_ms() });
                    if !done.result.outcome.is_success() && !cancel.is_cancelled() {
                        group.error.get_or_insert_with(|| format!("instance {i}: {}", done.result.error.as_deref().unwrap_or("execution cancelled")));
                        // Any non-user failure is a group error, even when an executor
                        // reports cancellation; siblings remain individually cancelled.
                        if done.result.outcome != StepOutcome::Cancelled || !group.outcomes.contains(&Some(StepOutcome::Error)) {
                            group.outcomes[i] = Some(StepOutcome::Error);
                        }
                        group.token.cancel();
                        group.cancel_pending(root, run, &done.name).await?;
                    }
                    group.progress(root, &run.run_id, &done.name, sink)?;
                } else {
                    record_step(done, &mut states, &mut outputs, &mut errors, root, run, sink).await;
                }
            }
        }
    }
    Ok(()) }.await;
    if let Err(error) = result {
        // Persistence failures must still cancel and reap live executors. Dropping
        // JoinSet here would abort their futures before subprocess cleanup.
        cancel.cancel();
        let mut error = error;
        while let Some(done) = tasks.join_next().await {
            match done {
                Ok(done) => {
                    if let Err(e) = crate::step_io::write_execution_artifacts(
                        root,
                        &run.run_id,
                        &done.name,
                        done.instance,
                        done.started_at_ms,
                        &done.result,
                    )
                    .await
                    {
                        error = error.context(format!("persist cancelled execution: {e:#}"));
                    }
                }
                Err(e) => error = error.context(format!("reap cancelled execution: {e}")),
            }
        }
        return Err(error);
    }
    Ok((states, errors, cancel.is_cancelled()))
}

fn failed(error: String) -> StepResult {
    StepResult {
        outcome: StepOutcome::Error,
        error: Some(error),
        output_text: String::new(),
        output_json: None,
        session_id: None,
    }
}
