use super::persistence;
use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_brain::execution;
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};

pub async fn handle(
    worker: &Worker,
    reference: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<RpcReply> {
    ensure!(valid_id(&reference.id), "invalid execution id");
    if action == "capability_probe" {
        let config = worker.configuration()?;
        let matches = opencoder_core::agent::scope::with_root_sync(
            config.agent.agents_dir.clone(),
            || -> Result<()> {
                for (name, expected) in input["agent_manifests"].as_object().into_iter().flatten() {
                    let actual = opencoder_core::brain::resources::agent_manifest(name)
                        .map_err(anyhow::Error::msg)?;
                    ensure!(
                        expected.as_str() == Some(actual.as_str()),
                        "pinned resource mismatch for {name}"
                    );
                }
                Ok(())
            },
        );
        return Ok(match matches {
            Ok(()) => RpcReply::ok(
                json!({"compatible":true,"features":["dag_dynamic_v1","brain_scheduler_v3","brain_scheduler_v4"]}),
            ),
            Err(error) => RpcReply::error(412, error.to_string()),
        });
    }
    if action == "notice_ack" {
        return super::outbox::ack(worker, reference, input).await;
    }
    if matches!(
        action,
        "scheduler_output" | "scheduler_summary" | "layered_output" | "layered_summary"
    ) {
        // The projection owner decides which reader serves the call: a v4 child
        // keeps its bounded output, a v3 child its scheduler output.
        let layered = worker
            .inner
            .journal
            .lock()
            .await
            .records
            .get(&reference.id)
            .is_some_and(|record| super::v4::parent::reports(&record.assignment.request.input));
        return if layered {
            super::v4::output::query(worker, reference, action, input).await
        } else {
            super::v3::output::query(worker, reference, action, input).await
        };
    }
    let schema_version = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&reference.id)
        .and_then(|record| record.assignment.request.input["schema_version"].as_u64());
    if schema_version == Some(3) {
        return super::v3::handle(worker, reference, action, input).await;
    }
    if schema_version == Some(4) {
        return super::v4::handle(worker, reference, action, input).await;
    }
    ensure!(
        reference.kind == ExecutionKind::Brain,
        "expected brain root"
    );
    let gate = worker.lifecycle_gate(&reference.id).await;
    let _guard = gate.lock().await;
    if action == "intent" {
        let journal = worker.inner.journal.lock().await;
        return Ok(journal
            .records
            .get(&reference.id)
            .map(|r| RpcReply::ok(r.assignment.request.input["brain_intent"].clone()))
            .unwrap_or_else(|| RpcReply::error(404, "run not found")));
    }
    let (previous, mut run) = match persistence::load(worker, &reference.id).await? {
        Some((record, run)) => (Some(record), run),
        None if matches!(action, "pause" | "cancel") => {
            let journal = worker.inner.journal.lock().await;
            let record = journal
                .records
                .get(&reference.id)
                .context("brain root not found")?;
            (
                None,
                execution::initialize(
                    &reference.id,
                    serde_json::from_value(record.assignment.request.input.clone())?,
                    persistence::now(),
                )?,
            )
        }
        None => return Ok(RpcReply::error(404, "brain run has not initialized")),
    };
    if (run.request.schema_version != 2
        || run
            .request
            .plan
            .as_ref()
            .is_some_and(|p| p.plan.schema_version != 2))
        && !matches!(
            action,
            "snapshot" | "context" | "events" | "actions" | "instances" | "instance"
        )
    {
        return Ok(RpcReply::error(409, opencoder_brain::graph::MIGRATION));
    }
    match action {
        "authorize" => {
            let receipt: ActionReceipt = serde_json::from_value(input)?;
            let current = run.actions.get(&receipt.id).context("unknown action")?;
            // Transport errors and acceptance results are mutable receipt fields.
            // An old outbox frame must retain the immutable action identity.
            let mut expected = current.clone();
            expected.state = receipt.state;
            expected.result = receipt.result.clone();
            expected.error = receipt.error.clone();
            if expected != receipt || receipt.state != ReceiptState::Prepared {
                return Ok(RpcReply::error(409, "action receipt identity changed"));
            }
            if current.state != ReceiptState::Prepared {
                return Ok(RpcReply::error(409, "action already accepted or settled"));
            }
            let instance = run
                .instances
                .get(&receipt.instance_id)
                .context("instance missing")?;
            ensure!(
                instance.attempt == receipt.attempt
                    && execution::fingerprint(&instance.inputs) == receipt.input_fingerprint,
                "action read set changed"
            );
            ensure!(
                if receipt.kind == ActionKind::Execute {
                    run.phase == RunPhase::Running
                } else {
                    run.phase == RunPhase::Cancelling
                },
                "action fenced by run control"
            );
            return Ok(RpcReply::ok(json!({"authorized":true})));
        }
        "snapshot" => {
            return Ok(RpcReply::ok(
                snapshot_locked(worker, &run, input["offset"].as_u64().unwrap_or(0) as usize)
                    .await?,
            ))
        }
        "context" => return Ok(RpcReply::ok(json!(execution::context(&mut run)))),
        "events" => {
            let page = worker
                .inner
                .state
                .store
                .todo_events_page(
                    &reference.id,
                    input["after"].as_i64().unwrap_or(0),
                    100,
                    512 * 1024,
                )
                .await?;
            return Ok(RpcReply::ok(json!({"events":page.events,"more":page.more})));
        }
        "actions" => {
            return Ok(RpcReply::ok(
                json!({"actions":run.actions.values().skip(input["offset"].as_u64().unwrap_or(0) as usize).take(100).collect::<Vec<_>>(),"total":run.actions.len()}),
            ))
        }
        "instances" => {
            let rows: Vec<_> = run
                .instances
                .values()
                .filter(|i| input["step"].as_str().is_none_or(|s| s == i.step_id))
                .collect();
            let offset = input["offset"].as_u64().unwrap_or(0) as usize;
            let items:Vec<_>=rows.iter().skip(offset).take(100).map(|i|json!({"id":i.id,"step_id":i.step_id,"status":i.status,"item_key":i.item_key,"execution":i.execution,"attempt":i.attempt})).collect();
            return Ok(RpcReply::ok(
                json!({"instances":items,"total":rows.len(),"next_offset":(offset+100<rows.len()).then_some(offset+100)}),
            ));
        }
        "instance" => {
            return Ok(run
                .instances
                .get(input["id"].as_str().unwrap_or(""))
                .map(|i| RpcReply::ok(json!(i)))
                .unwrap_or_else(|| RpcReply::error(404, "instance not found")))
        }
        "notice" => {
            let notice: BrainNotice = serde_json::from_value(input.clone())?;
            if !execution::apply_notice(&mut run, &notice)? {
                return Ok(RpcReply::ok(json!({"duplicate":true})));
            }
        }
        "receipt" => {
            let id = input["id"].as_str().context("missing action id")?;
            let reply: RpcReply = serde_json::from_value(input["reply"].clone())?;
            if run
                .actions
                .get(id)
                .is_some_and(|r| r.kind == ActionKind::Cancel)
            {
                if (200..300).contains(&reply.status) {
                    run.actions.get_mut(id).unwrap().state = ReceiptState::Accepted;
                }
            } else {
                execution::accept_action(&mut run, id, &reply, persistence::now())?;
            }
        }
        "published" => {
            let version: PlanVersion = serde_json::from_value(input.clone())?;
            if run.candidate_plan.is_none() && run.request.plan.as_ref() == Some(&version) {
                return Ok(RpcReply::ok(json!({"duplicate":true})));
            }
            let mut candidate = run
                .candidate_plan
                .take()
                .context("no pending plan publication")?;
            for (old, new) in candidate
                .plan
                .instances
                .iter_mut()
                .zip(&version.plan.instances)
            {
                if old.action.definition.is_none() {
                    old.action.definition = new.action.definition.clone();
                }
                if old.action.runtime.is_none() {
                    old.action.runtime = new.action.runtime.clone();
                }
                if old.action.codex.is_none() {
                    old.action.codex = new.action.codex.clone();
                }
                if old.action.agent_manifests.is_empty() {
                    old.action.agent_manifests = new.action.agent_manifests.clone();
                }
            }
            ensure!(
                candidate == version,
                "published plan differs from durable candidate"
            );
            execution::adopt(&mut run, version, persistence::now())?;
        }
        "publication_failed" => {
            ensure!(run.candidate_plan.is_some(), "no pending publication");
            run.candidate_plan = None;
            run.phase = RunPhase::Blocked;
            run.error = Some(
                input["error"]
                    .as_str()
                    .unwrap_or("plan publication failed")
                    .into(),
            );
            run.revision += 1;
        }
        "wake" => {
            if !run.phase.terminal() {
                run.revision += 1;
            }
        }
        "input" => execution::supply_input(
            &mut run,
            input["name"].as_str().context("missing input name")?,
            input["value"].clone(),
            persistence::now(),
        )?,
        "pause" | "resume" | "cancel" => execution::command(&mut run, action, persistence::now())?,
        _ => return Ok(RpcReply::error(400, "unknown brain operation")),
    }
    if previous.as_ref().is_none_or(|p| p.state_json != json!(run)) {
        persistence::save(worker, previous, &run, action, input).await?;
    }
    if run.phase.terminal() && !worker.inner.active.lock().await.contains_key(&reference.id) {
        let (status, result) = super::activate::outcome(&run);
        worker.inner.journal.lock().await.finalize(
            &reference.id,
            status,
            result,
            run.error.clone(),
        )?;
    }
    Ok(RpcReply::ok(
        json!({"revision":run.revision,"phase":run.phase}),
    ))
}

pub async fn snapshot(worker: &Worker, id: &str, offset: usize) -> Result<Value> {
    if let Some(snapshot) = worker.inner.state.store.brain_scheduler(id).await? {
        return Ok(json!(snapshot));
    }
    let gate = worker.lifecycle_gate(id).await;
    let _guard = gate.lock().await;
    let Some((_, run)) = persistence::load(worker, id).await? else {
        return Ok(json!({"initializing":true}));
    };
    snapshot_locked(worker, &run, offset).await
}

async fn snapshot_locked(worker: &Worker, run: &BrainRun, offset: usize) -> Result<Value> {
    let watermark = worker
        .inner
        .state
        .store
        .last_todo_event_seq(&run.id)
        .await?;
    let groups: Vec<_> = run.request.plan.iter().flat_map(|p| p.plan.instances.iter().map(|i|&i.id).chain(p.plan.steps.iter().map(|i|&i.id))).map(|step| {
        let mut counts = std::collections::BTreeMap::<String,usize>::new();
        for instance in run.instances.values().filter(|i|i.step_id == *step) { *counts.entry(serde_json::to_value(instance.status).unwrap().as_str().unwrap().into()).or_default() += 1; }
        let total: usize = counts.values().sum();
        json!({"id":*step,"counts":counts,"sealed":run.expansions.contains_key(step),"total":total,"current_instance":run.expansions.get(step).and_then(|ids|ids.last()),"active_visits":run.graph.tokens.keys().filter(|id|run.instances[*id].step_id == *step).collect::<Vec<_>>()})
    }).collect();
    let instances: Vec<_> = run.instances.values().skip(offset).take(100).map(|i|json!({"id":i.id,"step_id":i.step_id,"status":i.status,"attempt":i.attempt,"execution":i.execution,"node_id":i.node_id,"reason":i.reason,"item_key":i.item_key})).collect();
    Ok(
        json!({"id":run.id,"phase":run.phase,"revision":run.revision,"activation":run.activation,"handled_revision":run.handled_revision,"control_epoch":run.control_epoch,
        "objective":run.request.objective,"mode":run.request.mode,"plan":run.request.plan,"candidate_plan":run.candidate_plan,"input_requests":run.input_requests,"groups":groups,
        "instances":instances,"total_instances":run.instances.len(),"next_offset":(offset+100<run.instances.len()).then_some(offset+100),"watermark":watermark,
        "graph":run.graph,"deliverables":run.deliverables,"error":run.error,"created_at":run.created_at,"updated_at":run.updated_at}),
    )
}
