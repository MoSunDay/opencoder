use super::{
    config::{Inventory, RuntimeConfig},
    Host,
};
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::{NodeOperation, RpcReply};
use opencoder_store::fleet::handoff::RuntimeRecord;
use std::time::Duration;

impl Host {
    pub async fn runtime(&self, id: &str) -> Result<RuntimeRecord> {
        self.store
            .runtimes()
            .await?
            .into_iter()
            .find(|r| r.id == id)
            .context("runtime not registered")
    }

    pub async fn inventory(&self, runtime: &RuntimeRecord, wake: bool) -> Result<Inventory> {
        let config: RuntimeConfig = serde_json::from_value(runtime.config.clone())?;
        config.validate()?;
        let request = || {
            self.client
                .get(format!(
                    "{}/inventory",
                    config.endpoint.trim_end_matches('/')
                ))
                .bearer_auth(&self.token)
                .timeout(Duration::from_secs(15))
                .send()
        };
        let response = match request().await {
            Ok(response) => response,
            Err(error) if wake && error.is_connect() => {
                let status = tokio::process::Command::new("systemctl")
                    .arg("start")
                    .arg(&config.unit)
                    .status()
                    .await?;
                ensure!(status.success(), "could not wake runtime {}", runtime.id);
                let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
                loop {
                    match request().await {
                        Ok(response) => break response,
                        Err(error) => {
                            ensure!(
                                tokio::time::Instant::now() < deadline,
                                "runtime wake failed: {error}"
                            );
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
            Err(error) => return Err(error.into()),
        };
        let inventory: Inventory = response.error_for_status()?.json().await?;
        ensure!(
            inventory.runtime_id.as_deref() == Some(runtime.id.as_str()),
            "runtime endpoint returned another owner"
        );
        ensure!(
            inventory.registration.id == self.registration.id,
            "runtime changed node identity"
        );
        ensure!(
            inventory.registration.protocol_version == self.registration.protocol_version,
            "runtime protocol is incompatible"
        );
        if wake {
            self.store
                .put_definition("runtime_sleep", &runtime.id, &serde_json::Value::Null)
                .await?;
        }
        Ok(inventory)
    }

    pub async fn call_runtime(
        &self,
        runtime_id: &str,
        operation: &NodeOperation,
    ) -> Result<RpcReply> {
        let _creation = match self.creations.begin(runtime_id, operation) {
            Ok(creation) => creation,
            Err(reply) => return Ok(reply),
        };
        match tokio::time::timeout(
            super::admission::request_timeout(operation),
            self.forward_runtime(runtime_id, operation),
        )
        .await
        {
            Ok(reply) => reply,
            Err(_) => Ok(RpcReply::error(
                504,
                "runtime request timed out; retry using the same execution id",
            )),
        }
    }

    async fn forward_runtime(
        &self,
        runtime_id: &str,
        operation: &NodeOperation,
    ) -> Result<RpcReply> {
        let _use = self
            .store
            .shared_request_lock("runtime-use", runtime_id)
            .await?;
        let runtime = self.runtime(runtime_id).await?;
        let config: RuntimeConfig = serde_json::from_value(runtime.config.clone())?;
        let request = || {
            self.client
                .post(format!("{}/rpc", config.endpoint.trim_end_matches('/')))
                .bearer_auth(&self.token)
                .json(operation)
                .send()
        };
        let response = match request().await {
            Ok(response) => response,
            Err(error) if error.is_connect() => {
                self.inventory(&runtime, true).await?;
                request().await?
            }
            Err(error) => return Err(error.into()),
        };
        let reply = response.error_for_status()?.json().await?;
        if self
            .store
            .definition("runtime_sleep", runtime_id)
            .await?
            .is_some_and(|v| !v.is_null())
        {
            self.store
                .put_definition("runtime_sleep", runtime_id, &serde_json::Value::Null)
                .await?;
        }
        self.changes.send_modify(|n| *n += 1);
        Ok(reply)
    }

    pub async fn sync_inventory(&self) -> Result<Vec<opencoder_core::fleet::ExecutionIndex>> {
        let mut indexes = Vec::new();
        let mut active_loops = 0;
        let mut resource_errors = Vec::new();
        let mut runtime_ready = true;
        for runtime in self.store.runtimes().await? {
            if runtime.mode == "staged" {
                continue;
            }
            let _use = self
                .store
                .shared_request_lock("runtime-use", &runtime.id)
                .await?;
            let inventory = match self.inventory(&runtime, false).await {
                Ok(inventory) => {
                    if self
                        .store
                        .definition("runtime_sleep", &runtime.id)
                        .await?
                        .is_some_and(|v| !v.is_null())
                    {
                        self.store
                            .put_definition("runtime_sleep", &runtime.id, &serde_json::Value::Null)
                            .await?;
                    }
                    inventory
                }
                Err(error) => {
                    // Only an explicitly hibernated runtime may use its final
                    // inventory. A missing live runtime fails readiness.
                    if let Some(saved) = self
                        .store
                        .definition("runtime_sleep", &runtime.id)
                        .await?
                        .filter(|v| !v.is_null())
                    {
                        serde_json::from_value(saved)?
                    } else {
                        return Err(error);
                    }
                }
            };
            let ids: Vec<_> = inventory
                .indexes
                .iter()
                .map(|index| index.id.as_str())
                .collect();
            self.store
                .assign_runtime_inventory(&runtime.id, &ids)
                .await?;
            runtime_ready &= inventory.snapshot.ready;
            if let Some(error) = &inventory.snapshot.resource_error {
                resource_errors.push(format!("{}: {error}", runtime.id));
            }
            active_loops += inventory.snapshot.active_agent_loops;
            indexes.extend(inventory.indexes);
        }
        let capacity = self.store.capacity().await?;
        let ready = self
            .store
            .runtimes()
            .await?
            .iter()
            .any(|r| r.mode == "active");
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.active_agent_loops = active_loops;
        snapshot.active_runs = capacity.running;
        snapshot.pending_runs = capacity.queued;
        snapshot.max_runs = capacity.max_runs;
        snapshot.ready = ready && runtime_ready;
        snapshot.resource_error = (!resource_errors.is_empty()).then(|| resource_errors.join("; "));
        Ok(indexes)
    }
}
