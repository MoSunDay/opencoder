use super::Host;
use anyhow::{Context, Result};
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};

/// The wire operation already carries an execution reference on every query.
/// Never infer ownership from an ID prefix or from the currently active release.
fn execution_id(operation: &NodeOperation) -> Option<&str> {
    match operation {
        NodeOperation::Create { assignment } => Some(&assignment.index.id),
        NodeOperation::Brain { execution, .. }
        | NodeOperation::Inspect { execution }
        | NodeOperation::AcceptedRequest { execution }
        | NodeOperation::Command { execution, .. }
        | NodeOperation::Events { execution, .. }
        | NodeOperation::Messages { execution, .. }
        | NodeOperation::TodoItems { execution, .. }
        | NodeOperation::ProjectRuns { execution, .. }
        | NodeOperation::TeamTurns { execution, .. }
        | NodeOperation::DagInstances { execution, .. }
        | NodeOperation::DagInstanceEvents { execution, .. }
        | NodeOperation::DagSteps { execution, .. }
        | NodeOperation::DagStepEvents { execution, .. } => Some(&execution.id),
        NodeOperation::EventPayload { request } => Some(&request.execution.id),
        NodeOperation::DetailField { request } => Some(&request.execution.id),
        NodeOperation::Artifact { request } => Some(&request.execution.id),
        NodeOperation::Maintenance { command }
            if matches!(command.action.as_str(), "inspect" | "events" | "control") =>
        {
            command.input["execution"]["id"].as_str()
        }
        NodeOperation::Admission { .. } | NodeOperation::Maintenance { .. } => None,
    }
}

impl Host {
    async fn route(&self, operation: NodeOperation) -> Result<RpcReply> {
        if let NodeOperation::Admission { command } = &operation {
            let _lock = self.store.request_lock("host-admission", "cluster").await?;
            if *command != NodeAdmissionCommand::Status {
                self.store
                    .put_definition("host", "admission", &json!(command))
                    .await?;
            }
            let mut active_runs = 0u64;
            let mut owned_processes = 0u64;
            for runtime in self.store.runtimes().await? {
                if runtime.mode == "staged" {
                    continue;
                }
                if *command == NodeAdmissionCommand::Status
                    && self
                        .store
                        .definition("runtime_sleep", &runtime.id)
                        .await?
                        .is_some_and(|v| !v.is_null())
                {
                    continue;
                }
                let reply = self.call_runtime(&runtime.id, &operation).await?;
                if reply.status >= 300 {
                    return Ok(reply);
                }
                active_runs += reply.body["active_runs"]
                    .as_u64()
                    .context("runtime admission missing active runs")?;
                owned_processes += reply.body["owned_processes"]
                    .as_u64()
                    .context("runtime admission missing owned processes")?;
            }
            let saved = self.store.definition("host", "admission").await?;
            return Ok(RpcReply::ok(
                json!({"mode":if saved == Some(json!(NodeAdmissionCommand::Freeze)) { "frozen" } else { "open" },"active_runs":active_runs,"owned_processes":owned_processes}),
            ));
        }
        if let NodeOperation::Maintenance { command } = &operation {
            match command.action.as_str() {
                "ask" => {
                    let id = command.input["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("maintenance-{}", ulid::Ulid::new()));
                    let prompt = command.input["prompt"].as_str().unwrap_or("");
                    if prompt.trim().is_empty() || !id.starts_with("maintenance-") {
                        return Ok(RpcReply::error(400, "maintenance id and prompt required"));
                    }
                    let request = CreateExecution {
                        id: id.clone(),
                        kind: ExecutionKind::Maintenance,
                        target: Some("act".into()),
                        input: json!({"prompt":prompt}),
                        node_id: Some(self.registration.id.clone()),
                    };
                    let index = ExecutionIndex {
                        id,
                        node_id: self.registration.id.clone(),
                        kind: ExecutionKind::Maintenance,
                        created_at: opencoder_core::message::now_ms(),
                        status: ExecutionStatus::Pending,
                    };
                    return Box::pin(self.route(NodeOperation::Create {
                        assignment: Assignment {
                            request,
                            index,
                            definition: None,
                            runtime: None,
                            codex: None,
                        },
                    }))
                    .await;
                }
                "host_handoff_ready" => {
                    let server = command.input["server"]
                        .as_str()
                        .context("server identity required")?;
                    self.store
                        .put_definition("host_ack", server, &json!({"instance":self.instance}))
                        .await?;
                    return Ok(RpcReply::ok(json!({"acknowledged":true})));
                }
                "status" => return Ok(RpcReply::ok(self.status().await?)),
                "scheduling" => {
                    // multi-runtime hosts run sessions in per-runtime workdirs,
                    // so node-level scheduling workdir never applies here.
                    return Ok(RpcReply::ok(
                        json!({"max_runs":self.store.capacity().await?.max_runs,"queue_order":"fifo","workdir":null,"workdir_supported":false}),
                    ));
                }
                "dialogs_clear" => {
                    // Fan out across every runtime the host owns: live ones
                    // delete for real (sessions + journal records), while a
                    // hibernated runtime is only represented by its saved
                    // final inventory — dropping the rows there is what
                    // stops the next sync from resurrecting the dialogs.
                    let kind = match command.input.get("kind") {
                        None => ExecutionKind::Operator,
                        Some(value) => match serde_json::from_value::<ExecutionKind>(value.clone())
                        {
                            Ok(kind @ (ExecutionKind::Operator | ExecutionKind::Agent)) => kind,
                            Ok(_) => {
                                return Ok(RpcReply::error(
                                    400,
                                    "dialogs_clear only supports operator or agent",
                                ))
                            }
                            Err(error) => {
                                return Ok(RpcReply::error(
                                    400,
                                    format!("invalid dialogs_clear kind: {error}"),
                                ))
                            }
                        },
                    };
                    let requested: Vec<String> = command.input["sessions"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut removed = 0u64;
                    let mut forgotten = 0usize;
                    let mut skipped: Vec<String> = Vec::new();
                    for runtime in self.store.runtimes().await? {
                        if runtime.mode == "staged" {
                            continue;
                        }
                        let sleeping = self
                            .store
                            .definition("runtime_sleep", &runtime.id)
                            .await?
                            .is_some_and(|v| !v.is_null());
                        if !sleeping {
                            let reply = self.call_runtime(&runtime.id, &operation).await?;
                            if reply.status >= 300 {
                                return Ok(reply);
                            }
                            removed += reply.body["removed"].as_u64().unwrap_or(0);
                            forgotten += reply.body["forgotten"].as_u64().unwrap_or(0) as usize;
                            if let Some(rows) = reply.body["skipped"].as_array() {
                                for value in rows {
                                    if let Some(id) = value.as_str() {
                                        if !skipped.iter().any(|known| known == id) {
                                            skipped.push(id.to_owned());
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        let saved = self
                            .store
                            .definition("runtime_sleep", &runtime.id)
                            .await?
                            .unwrap_or(serde_json::Value::Null);
                        let mut saved: serde_json::Value = serde_json::from_value(saved)?;
                        let Some(rows) = saved
                            .get_mut("indexes")
                            .and_then(serde_json::Value::as_array_mut)
                        else {
                            continue;
                        };
                        rows.retain(|index| {
                            let Some(id) = index.get("id").and_then(serde_json::Value::as_str)
                            else {
                                return true;
                            };
                            if !requested.iter().any(|asked| asked == id)
                                || index.get("kind").and_then(Value::as_str) != Some(kind.prefix())
                            {
                                return true;
                            }
                            let live = matches!(
                                index.get("status").and_then(serde_json::Value::as_str),
                                Some("pending" | "running" | "cancelling" | "interrupted")
                            );
                            if live {
                                if !skipped.iter().any(|known| known == id) {
                                    skipped.push(id.to_owned());
                                }
                                return true;
                            }
                            removed += 1;
                            false
                        });
                        self.store
                            .put_definition("runtime_sleep", &runtime.id, &saved)
                            .await?;
                    }
                    return Ok(opencoder_core::fleet::RpcReply::ok(
                        json!({"ok":true,"kind":kind,"removed":removed,"skipped":skipped,"forgotten":forgotten}),
                    ));
                }
                "configure_scheduling" => {
                    let settings: NodeScheduling =
                        serde_json::from_value::<NodeScheduling>(command.input.clone())?
                            .normalized();
                    settings.validate().map_err(anyhow::Error::msg)?;
                    // Capability violations are client errors (400), not host
                    // routing failures; keep parity with the runtime side.
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
                    self.store.configure_capacity(settings.max_runs).await?;
                    return Ok(RpcReply::ok(json!(settings)));
                }
                _ => {}
            }
        }
        let runtime = if let NodeOperation::Create { assignment } = &operation {
            let parent = assignment.request.input["_brain"]["parent"]["run_id"].as_str();
            let inherited = match parent {
                Some(id) => Some(
                    self.store
                        .owner(id)
                        .await?
                        .context("parent runtime not found")?
                        .runtime_id,
                ),
                None => None,
            };
            self.store
                .assign_runtime(&assignment.index.id, inherited.as_deref())
                .await?
        } else if let Some(id) = execution_id(&operation) {
            match self.store.owner(id).await? {
                Some(owner) => owner.runtime_id,
                None if matches!(&operation, NodeOperation::Brain { action, .. } if action == "capability_probe") => {
                    self.active_runtime().await?
                }
                None => {
                    return Ok(RpcReply::error(
                        404,
                        "execution runtime ownership not found",
                    ))
                }
            }
        } else {
            self.active_runtime().await?
        };
        self.call_runtime(&runtime, &operation).await
    }

    pub async fn active_runtime(&self) -> Result<String> {
        self.store
            .runtimes()
            .await?
            .into_iter()
            .find(|r| r.mode == "active")
            .map(|r| r.id)
            .context("no active runtime")
    }

    pub async fn status(&self) -> Result<Value> {
        let mut runtimes = Vec::new();
        for runtime in self.store.runtimes().await? {
            runtimes.push(
                json!({"runtime":runtime,"remaining":self.store.runtime_tickets(&runtime.id).await?,
                "hibernated":self.store.definition("runtime_sleep", &runtime.id).await?.is_some_and(|v| !v.is_null()),
                "collection":self.store.definition("runtime_gc", &runtime.id).await?}),
            );
        }
        Ok(
            json!({"node":self.registration,"snapshot":self.snapshot(),"capacity":self.store.capacity().await?,"runtimes":runtimes}),
        )
    }
}

#[async_trait::async_trait]
impl NodeService for Host {
    async fn reconnect_allowed(&self, remote: &str) -> Result<bool> {
        if self.retiring() {
            return Ok(false);
        }
        let servers = self.store.definitions("release_server").await?;
        Ok(servers.is_empty()
            || servers
                .iter()
                .any(|s| s["enabled"] == true && s["url"] == remote))
    }
    fn retiring(&self) -> bool {
        self.retiring.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn registration(&self) -> NodeRegistration {
        self.registration.clone()
    }
    fn snapshot(&self) -> NodeSnapshot {
        let mut snapshot = self.snapshot.read().unwrap().clone();
        snapshot.sequence = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        snapshot
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }
    async fn report(&self) -> Result<opencoder_node::fleet::NodeReport> {
        let _gate = self.report_gate.lock().await;
        let records = self.indexes().await?;
        Ok(opencoder_node::fleet::NodeReport {
            snapshot: self.snapshot(),
            records,
            brain: self.brain_frames().await?,
        })
    }
    async fn indexes(&self) -> Result<Vec<ExecutionIndex>> {
        match self.sync_inventory().await {
            Ok(indexes) => Ok(indexes),
            Err(error) => {
                let mut snapshot = self.snapshot.write().unwrap();
                snapshot.ready = false;
                snapshot.resource_error = Some(error.to_string());
                Err(error)
            }
        }
    }
    async fn brain_frames(&self) -> Result<Vec<NodeFrame>> {
        let mut frames = Vec::new();
        for runtime in self.store.runtimes().await? {
            let _use = self
                .store
                .shared_request_lock("runtime-use", &runtime.id)
                .await?;
            if runtime.mode == "staged"
                || self
                    .store
                    .definition("runtime_sleep", &runtime.id)
                    .await?
                    .is_some_and(|v| !v.is_null())
            {
                continue;
            }
            let config: super::config::RuntimeConfig = serde_json::from_value(runtime.config)?;
            let mut batch: Vec<NodeFrame> = self
                .client
                .get(format!("{}/frames", config.endpoint.trim_end_matches('/')))
                .bearer_auth(&self.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            frames.append(&mut batch);
        }
        Ok(frames)
    }
    async fn handle(&self, operation: NodeOperation) -> RpcReply {
        match self.route(operation).await {
            Ok(reply) => reply,
            Err(error) => RpcReply::error(503, format!("host routing: {error:#}")),
        }
    }
}
