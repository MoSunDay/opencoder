use crate::Worker;
use anyhow::Result;
use opencoder_core::{fleet::*, message::now_ms};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) async fn run(worker: &Worker, command: ExecutionCommand) -> Result<RpcReply> {
    opencoder_core::agent::scope::with_root(None, run_unscoped(worker, command)).await
}
async fn run_unscoped(worker: &Worker, command: ExecutionCommand) -> Result<RpcReply> {
    match command.action.as_str() {
        "scheduling" => {
            if let Some(host) = &worker.inner.host_capacity {
                // Same capability flag as the host side: no node-level workdir.
                return Ok(RpcReply::ok(
                    json!({"max_runs":host.store.capacity().await?.max_runs,"queue_order":"fifo","workdir":null,"workdir_supported":false}),
                ));
            }
            Ok(RpcReply::ok(json!(worker.inner.scheduling.get())))
        }
        "configure_scheduling" => {
            let settings: NodeScheduling =
                match serde_json::from_value::<NodeScheduling>(command.input) {
                    Ok(settings) => settings.normalized(),
                    Err(error) => return Ok(RpcReply::error(400, error.to_string())),
                };
            if let Err(error) = settings.validate() {
                return Ok(RpcReply::error(400, error));
            }
            if let Some(host) = &worker.inner.host_capacity {
                if settings.queue_order != QueueOrder::Fifo {
                    return Ok(RpcReply::error(
                        400,
                        "multi-runtime hosts require FIFO ordering",
                    ));
                }
                if settings.workdir.is_some() {
                    return Ok(RpcReply::error(
                        400,
                        "multi-runtime hosts do not support a scheduling workdir",
                    ));
                }
                host.store.configure_capacity(settings.max_runs).await?;
                return Ok(RpcReply::ok(json!(settings)));
            }
            let _gate = worker.inner.admission.clone().lock_owned().await;
            if let Some(dir) = &settings.workdir {
                std::fs::create_dir_all(dir)
                    .map_err(|error| anyhow::anyhow!("scheduling workdir unavailable: {error}"))?;
            }
            worker.inner.scheduling.save(settings.clone())?;
            let _gate = super::queue::dispatch_owned(worker, _gate).await?;
            opencoder_session::loop_registry::notify_change();
            Ok(RpcReply::ok(json!(settings)))
        }
        "status" => Ok(RpcReply::ok(
            json!({"node":worker.registration(),"snapshot":worker.snapshot()}),
        )),
        "executions" => Ok(RpcReply::ok(json!({"executions":worker.indexes().await?}))),
        "inspect" => {
            let execution = match parse_execution_ref(&command.input) {
                Ok(execution) => execution,
                Err(reply) => return Ok(reply),
            };
            super::query::inspect(worker, &execution).await
        }
        "events" => {
            let execution = match parse_execution_ref(&command.input) {
                Ok(execution) => execution,
                Err(reply) => return Ok(reply),
            };
            super::query::events(
                worker,
                &execution,
                command.input["after"].as_i64().unwrap_or(0),
            )
            .await
        }
        "control" => {
            let execution = match parse_execution_ref(&command.input) {
                Ok(execution) => execution,
                Err(reply) => return Ok(reply),
            };
            let control: ExecutionCommand =
                serde_json::from_value(command.input["command"].clone())?;
            super::command::command(worker, &execution, control).await
        }
        "config" => super::native(worker, "GET", "/api/config", Value::Null).await,
        "configure" => {
            let _gate = worker.inner.admission.clone().lock_owned().await;
            if let Some(error) = worker.admission_error() {
                return Ok(RpcReply::error(503, error));
            }
            super::native(worker, "PATCH", "/api/config", command.input).await
        }
        "models" => super::native(worker, "GET", "/api/models", Value::Null).await,
        "skills" => super::native(worker, "GET", "/api/skills", Value::Null).await,
        "resources" => {
            crate::resources::check_mount(worker.configuration()?.agent.agents_dir.as_deref())?;
            Ok(RpcReply::ok(
                json!({"agents":opencoder_core::agent::list_agents(),"read_only":true}),
            ))
        }
        // Bulk-clear console dialogs. `input.sessions` carries the ids the
        // control plane decided are safe to drop (terminal index rows), and
        // `input.kind` fences the request to one public chat lane. The node
        // rechecks both the durable journal kind and lifecycle while holding
        // the admission gate, so a stale control-plane snapshot cannot delete
        // a newly running or differently typed execution.
        "dialogs_clear" => {
            let kind = match dialog_kind(&command.input) {
                Ok(kind) => kind,
                Err(reply) => return Ok(reply),
            };
            let requested: Vec<String> = command.input["sessions"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let _admission = worker.inner.admission.lock().await;
            let active: HashSet<String> =
                worker.inner.active.lock().await.keys().cloned().collect();
            let journal = worker.inner.journal.lock().await;
            let mut skipped = Vec::new();
            let mut deletable = Vec::new();
            for id in &requested {
                // A session without a top-level journal is an internal child
                // or an already-cleared id. It is intentionally ignored so a
                // public bulk clear cannot erase a parent-owned transcript.
                let Some(record) = journal.records.get(id) else {
                    continue;
                };
                if record.assignment.index.kind != kind {
                    return Ok(RpcReply::error(
                        409,
                        format!(
                            "session {id} belongs to {} lane",
                            record.assignment.index.kind.prefix()
                        ),
                    ));
                }
                if active.contains(id)
                    || !matches!(
                        record.assignment.index.status,
                        ExecutionStatus::Idle
                            | ExecutionStatus::Done
                            | ExecutionStatus::Error
                            | ExecutionStatus::Cancelled
                    )
                {
                    skipped.push(id.clone());
                } else {
                    deletable.push(id.clone());
                }
            }
            drop(journal);
            let removed = worker.inner.state.store.delete_sessions(&deletable).await?;
            let mut forgotten = 0usize;
            {
                let mut journal = worker.inner.journal.lock().await;
                for id in &deletable {
                    if journal.forget(id).unwrap_or(false) {
                        forgotten += 1;
                    }
                }
            }
            Ok(RpcReply::ok(
                json!({"ok": true, "kind": kind, "removed": removed, "skipped": skipped, "forgotten": forgotten}),
            ))
        }
        "ask" => {
            let id = command.input["id"]
                .as_str()
                .filter(|s| s.starts_with("maintenance-"))
                .map(str::to_owned)
                .unwrap_or_else(|| format!("maintenance-{}", ulid::Ulid::new()));
            let prompt = command.input["prompt"].as_str().unwrap_or("");
            if prompt.trim().is_empty() {
                return Ok(RpcReply::error(400, "maintenance prompt required"));
            }
            let index = ExecutionIndex {
                id: id.clone(),
                created_at: now_ms(),
                kind: ExecutionKind::Maintenance,
                node_id: worker.inner.registration.id.clone(),
                status: ExecutionStatus::Pending,
            };
            let request = CreateExecution {
                id,
                kind: ExecutionKind::Maintenance,
                target: Some("act".into()),
                input: json!({"prompt":prompt}),
                node_id: Some(worker.inner.registration.id.clone()),
            };
            super::create::create(
                worker,
                Assignment {
                    runtime: None,
                    codex: None,
                    index,
                    request,
                    definition: None,
                },
            )
            .await
        }
        _ => Ok(RpcReply::error(400, "unknown maintenance operation")),
    }
}

fn dialog_kind(input: &Value) -> std::result::Result<ExecutionKind, RpcReply> {
    let Some(value) = input.get("kind") else {
        // Older control binaries omitted the selector; their endpoint always
        // meant the legacy Operator lane.
        return Ok(ExecutionKind::Operator);
    };
    let kind = serde_json::from_value::<ExecutionKind>(value.clone())
        .map_err(|error| RpcReply::error(400, format!("invalid dialogs_clear kind: {error}")))?;
    if !matches!(kind, ExecutionKind::Operator | ExecutionKind::Agent) {
        return Err(RpcReply::error(
            400,
            "dialogs_clear only supports operator or agent",
        ));
    }
    Ok(kind)
}

fn parse_execution_ref(input: &Value) -> std::result::Result<ExecutionRef, RpcReply> {
    serde_json::from_value(input["execution"].clone()).map_err(|error| {
        RpcReply::error(400, format!("typed execution reference required: {error}"))
    })
}
