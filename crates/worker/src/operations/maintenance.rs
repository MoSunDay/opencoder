use crate::Worker;
use anyhow::Result;
use opencoder_core::{fleet::*, message::now_ms};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};

pub(super) async fn run(worker: &Worker, command: ExecutionCommand) -> Result<RpcReply> {
    opencoder_core::agent::scope::with_root(None, run_unscoped(worker, command)).await
}
async fn run_unscoped(worker: &Worker, command: ExecutionCommand) -> Result<RpcReply> {
    match command.action.as_str() {
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
            let _gate = worker.inner.admission.lock().await;
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

fn parse_execution_ref(input: &Value) -> std::result::Result<ExecutionRef, RpcReply> {
    serde_json::from_value(input["execution"].clone()).map_err(|error| {
        RpcReply::error(400, format!("typed execution reference required: {error}"))
    })
}
