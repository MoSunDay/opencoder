use super::{config::RuntimeConfig, Host};
use anyhow::{ensure, Result};

impl Host {
    pub async fn hibernate(&self, id: &str) -> Result<()> {
        let _activation = self.store.request_lock("release", "activation").await?;
        // A long RPC must not let idle collection retain the fleet-wide
        // activation lock. Retry collection after readers release ownership.
        let _use = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            self.store.request_lock("runtime-use", id),
        )
        .await
        .map_err(|_| anyhow::anyhow!("runtime has in-flight requests; retry hibernation"))??;
        let runtime = self.runtime(id).await?;
        ensure!(
            runtime.mode == "retired",
            "only retired runtimes may hibernate"
        );
        ensure!(
            self.store.runtime_tickets(id).await?.is_empty(),
            "runtime still owns queued or running reservations"
        );
        let inventory = self.inventory(&runtime, false).await?;
        ensure!(
            inventory.can_hibernate,
            "runtime has live executions, tools or writes"
        );
        self.store
            .put_definition("runtime_sleep", id, &serde_json::to_value(&inventory)?)
            .await?;
        let config: RuntimeConfig = serde_json::from_value(runtime.config)?;
        let status = tokio::process::Command::new("systemctl")
            .arg("stop")
            .arg(&config.unit)
            .status()
            .await?;
        ensure!(status.success(), "runtime unit did not stop cleanly");
        Ok(())
    }

    pub async fn handoff_acknowledged(&self, current: &serde_json::Value) -> Result<bool> {
        if current["ingress"] != current["instance"] {
            return Ok(false);
        }
        for server in self.store.definitions("release_server").await? {
            if server["enabled"] != true {
                continue;
            }
            let id = server["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("server identity missing"))?;
            let ack = self
                .store
                .definition("host_ack", id)
                .await?
                .unwrap_or_default();
            if ack["instance"] != current["instance"] {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn collect_retired(&self) -> Result<()> {
        let current = self
            .store
            .definition("host", "current")
            .await?
            .unwrap_or_default();
        if current["instance"] != self.instance {
            return Ok(());
        }
        for runtime in self.store.runtimes().await? {
            if runtime.mode != "retired"
                || !self.store.runtime_tickets(&runtime.id).await?.is_empty()
                || self
                    .store
                    .definition("runtime_sleep", &runtime.id)
                    .await?
                    .is_some_and(|v| !v.is_null())
            {
                continue;
            }
            match self.inventory(&runtime, false).await {
                Ok(inventory) if inventory.can_hibernate => {
                    let result = self.hibernate(&runtime.id).await;
                    let status = match result {
                        Ok(()) => serde_json::json!({"state":"hibernated"}),
                        Err(error) => {
                            serde_json::json!({"state":"failed","error":error.to_string()})
                        }
                    };
                    self.store
                        .put_definition("runtime_gc", &runtime.id, &status)
                        .await?;
                }
                Ok(_) => {}
                Err(error) => {
                    self.store
                        .put_definition(
                            "runtime_gc",
                            &runtime.id,
                            &serde_json::json!({"state":"failed","error":error.to_string()}),
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }
}
