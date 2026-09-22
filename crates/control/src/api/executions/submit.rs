use crate::AppState;
use opencoder_core::{fleet::*, message::now_ms};
use opencoder_store::fleet::handoff::dispatch_key;
use serde_json::Value;
use std::sync::Arc;

pub async fn submit(state: &Arc<AppState>, request: CreateExecution) -> RpcReply {
    submit_private(state, request, None).await
}

pub(crate) async fn submit_private(
    state: &Arc<AppState>,
    request: CreateExecution,
    private_context: Option<PrivateExecutionContext>,
) -> RpcReply {
    if let Err(message) = super::private_context::validate(&request, private_context.as_ref()) {
        return RpcReply::error(400, message);
    }
    if request.kind == ExecutionKind::System {
        return RpcReply::error(
            400,
            "system team execution is retired; use explicit node maintenance",
        );
    }
    if request.kind == ExecutionKind::Project
        && request.id != format!("project-{}", request.target.as_deref().unwrap_or(""))
    {
        return RpcReply::error(
            400,
            "project execution id must be project-<todo id> to preserve plan/act affinity",
        );
    }
    // Agent-kind sessions carry the workflow-declared `how_append` payload
    // as a DAG agent step. When the chat creates a fresh Agent session, the
    // first prompt is also the initial how entry; validate that derived value
    // here so an oversized request fails before placement.
    if request.kind == ExecutionKind::Agent {
        let text = match request.input.get("how_append") {
            None | Some(Value::Null) => request.input.get("prompt").and_then(Value::as_str),
            Some(Value::String(text)) => Some(text.as_str()),
            Some(other) => {
                return RpcReply::error(400, format!("how_append must be a string, got {other}"));
            }
        };
        if let Some(text) = text {
            if text.len() > opencoder_dag::spec::MAX_HOW_APPEND_BYTES {
                return RpcReply::error(
                    400,
                    format!(
                        "how_append exceeds {} bytes (got {})",
                        opencoder_dag::spec::MAX_HOW_APPEND_BYTES,
                        text.len()
                    ),
                );
            }
        }
    }
    if let Err(error) = request.validate() {
        return RpcReply::error(400, error);
    }
    match submit_inner(state, request, private_context).await {
        Ok(reply) => reply,
        Err(error) => RpcReply::error(500, format!("submit execution: {error:#}")),
    }
}

async fn submit_inner(
    state: &Arc<AppState>,
    request: CreateExecution,
    private_context: Option<PrivateExecutionContext>,
) -> anyhow::Result<RpcReply> {
    let _request_lock = state.fleet.request_lock("execution", &request.id).await?;
    let key = dispatch_key(&request).to_owned();
    let fingerprint = if private_context.is_some() {
        opencoder_core::token_hash(&serde_json::to_string(&(&request, &private_context))?)
    } else {
        opencoder_core::token_hash(&serde_json::to_string(&request)?)
    };
    if !state
        .fleet
        .claim_request("execution", &key, &fingerprint)
        .await?
    {
        return Ok(RpcReply::error(
            409,
            "execution id already used with different input",
        ));
    }
    if let Some(receipt) = state.fleet.receipt("execution", &key).await? {
        if matches!(receipt.phase.as_str(), "accepted" | "rejected") {
            return Ok(serde_json::from_value(receipt.payload)?);
        }
    }
    let mut frozen = state.fleet.assignment(&request.id).await?;
    if let Some(old) = &frozen {
        if old.request != request || old.private_context != private_context {
            let rejected = state
                .fleet
                .receipt("execution", dispatch_key(&old.request))
                .await?
                .is_some_and(|receipt| receipt.phase == "rejected");
            if request.kind != ExecutionKind::Project || !rejected {
                return Ok(RpcReply::error(
                    409,
                    "previous execution dispatch is unresolved or accepted",
                ));
            }
            frozen = None;
        }
    }
    let (_permit, assignment) = if let Some(assignment) = frozen {
        (None, assignment)
    } else {
        let _gate = state.placement.lock().await;
        let permit = match state.admission.enter().await {
            Ok(permit) => permit,
            Err(error) => return Ok(RpcReply::error(503, error)),
        };
        if let Some(index) = state.fleet.index(&request.id).await? {
            if index.kind != request.kind {
                return Ok(RpcReply::error(
                    409,
                    "execution id is already assigned to another kind",
                ));
            }
            if request
                .node_id
                .as_deref()
                .is_some_and(|node| node != index.node_id)
            {
                return Ok(RpcReply::error(
                    409,
                    "execution is already assigned to another node",
                ));
            }
            // The node compares the original request and returns its durable
            // acceptance. A newer definition must not replace its snapshot.
            let definition = if request.kind == ExecutionKind::Project || private_context.is_some()
            {
                match crate::api::catalog::resolve(state, &request).await {
                    Ok(definition) => definition,
                    Err(reply) => return Ok(reply),
                }
            } else {
                None
            };
            let assignment = Assignment {
                private_context: private_context.clone(),
                runtime: crate::api::settings::registered::snapshot(state).await?,
                codex: crate::api::settings::codex(state).await?,
                index,
                request,
                definition,
            };
            (Some(permit), assignment)
        } else {
            let definition = match crate::api::catalog::resolve(state, &request).await {
                Ok(definition) => definition,
                Err(reply) => return Ok(reply),
            };
            let mut nodes = Vec::new();
            for node in state.hub.views().await {
                if state.admission.node_allowed(&node).await {
                    nodes.push(node);
                }
            }
            let mut incompatibilities = Vec::new();
            let node = loop {
                let Some(node) =
                    select_queue_node(&nodes, request.kind, request.node_id.as_deref(), now_ms())
                        .cloned()
                else {
                    return Ok(RpcReply::error(
                        503,
                        if incompatibilities.is_empty() {
                            "no ready online node can accept this execution".to_string()
                        } else {
                            format!(
                                "no ready compatible node can accept this execution: {}",
                                incompatibilities.join("; ")
                            )
                        },
                    ));
                };
                let action = request.input.get("_brain").and_then(|b| b.get("action"));
                let mut required = super::capabilities::required(&request, definition.as_ref());
                if private_context.is_some() {
                    required.push(opencoder_core::fleet::private_files::CAPABILITY);
                }
                if action.is_some() || !required.is_empty() {
                    if let Err(reply) = super::capabilities::probe(
                        state,
                        &node.registration.id,
                        ExecutionRef {
                            id: request.id.clone(),
                            kind: request.kind,
                        },
                        action.cloned().unwrap_or_else(|| serde_json::json!({})),
                        &required,
                    )
                    .await
                    {
                        incompatibilities.push(format!("{}: {}", node.registration.id, reply.body));
                        nodes.retain(|n| n.registration.id != node.registration.id);
                        continue;
                    }
                }
                break node;
            };
            let index = ExecutionIndex {
                id: request.id.clone(),
                created_at: now_ms(),
                kind: request.kind,
                node_id: node.registration.id.clone(),
                status: ExecutionStatus::Pending,
            };
            let assignment = Assignment {
                private_context: private_context.clone(),
                runtime: crate::api::settings::registered::snapshot(state).await?,
                codex: crate::api::settings::codex(state).await?,
                index,
                request,
                definition,
            };
            (Some(permit), assignment)
        }
    };
    if let Err(message) = super::private_context::validate_definition(
        assignment.private_context.as_ref(),
        assignment.definition.as_ref(),
    ) {
        return Ok(RpcReply::error(409, message));
    }
    state.hub.reserve(&assignment.index).await;
    state
        .fleet
        .prepare_assignment(&assignment, &fingerprint)
        .await?;
    let index = assignment.index.clone();
    let mut reply = state
        .hub
        .call(
            &index.node_id,
            NodeOperation::Create {
                assignment: assignment.clone(),
            },
        )
        .await;
    if reply.status == 428 {
        let definition = match crate::api::catalog::resolve(state, &assignment.request).await {
            Ok(definition) => definition,
            Err(reply) => return Ok(reply),
        };
        if let Err(message) = super::private_context::validate_definition(
            assignment.private_context.as_ref(),
            definition.as_ref(),
        ) {
            return Ok(RpcReply::error(409, message));
        }
        state.hub.reserve(&index).await;
        reply = state
            .hub
            .call(
                &index.node_id,
                NodeOperation::Create {
                    assignment: Assignment {
                        private_context: private_context.clone(),
                        definition,
                        ..assignment
                    },
                },
            )
            .await;
    }
    if (200..300).contains(&reply.status) {
        let accepted: ExecutionIndex = serde_json::from_value(reply.body.clone())?;
        if accepted.id != index.id
            || accepted.node_id != index.node_id
            || accepted.created_at != index.created_at
            || accepted.kind != index.kind
        {
            anyhow::bail!("invalid node acceptance");
        }
        let reply = RpcReply {
            status: 202,
            body: {
                let mut body = serde_json::to_value(accepted)?;
                if index.kind == ExecutionKind::Project {
                    if let Some(id) = reply.body.get("run_id") {
                        body["run_id"] = id.clone();
                    }
                }
                body
            },
        };
        state
            .fleet
            .finish_dispatch(&key, &fingerprint, &reply)
            .await?;
        return Ok(reply);
    }
    // The socket settles actual replies and complete reports own status. A
    // timeout/disconnect retains Pending because the node may have accepted it.
    if (400..500).contains(&reply.status) && !matches!(reply.status, 408 | 425 | 428 | 429) {
        state
            .fleet
            .save_receipt(
                "execution",
                &key,
                &opencoder_store::fleet::handoff::Receipt {
                    fingerprint,
                    phase: "rejected".into(),
                    payload: serde_json::to_value(&reply)?,
                },
            )
            .await?;
    }
    Ok(reply)
}
