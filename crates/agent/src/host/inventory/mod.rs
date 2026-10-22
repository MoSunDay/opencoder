mod fetch;
use super::{config::Inventory, Host};
use anyhow::{ensure, Result};
use opencoder_core::fleet::{inventory::*, *};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct HostInventory {
    log: InventoryLog,
    runtimes: BTreeMap<String, RuntimeInventory>,
    checked: Option<Instant>,
}
#[derive(Default)]
struct RuntimeInventory {
    generation: Option<String>,
    revision: Option<u64>,
    sleep_revision: Option<String>,
    legacy: bool,
    ids: BTreeSet<String>,
    snapshot: Option<NodeSnapshot>,
}

impl Host {
    pub(super) async fn collect_inventory_changes(
        &self,
        after: Option<u64>,
    ) -> Result<InventoryChanges> {
        let mut cache = self.inventory.lock().await;
        let result = cache.refresh(self).await;
        if let Err(error) = result {
            for runtime in cache.runtimes.values_mut() {
                runtime.revision = None;
                runtime.generation = None;
                runtime.sleep_revision = None;
            }
            let mut snapshot = self.snapshot.write().unwrap();
            snapshot.ready = false;
            snapshot.resource_error = Some(error.to_string());
            return Err(error);
        }
        Ok(cache.log.read(after))
    }
}

impl HostInventory {
    async fn refresh(&mut self, host: &Host) -> Result<()> {
        let reconcile = self
            .checked
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(600));
        let runtimes = host.store.runtimes().await?;
        let mut current = BTreeSet::new();
        let mut active_loops = 0;
        let mut ready = false;
        let mut runtime_ready = true;
        let mut errors = Vec::new();
        for runtime in &runtimes {
            if runtime.mode == "staged" {
                continue;
            }
            current.insert(runtime.id.clone());
            let _use = host
                .store
                .shared_request_lock("runtime-use", &runtime.id)
                .await?;
            let (stamp, asleep) = host.store.runtime_sleep_stamp(&runtime.id).await?;
            let cache = self.runtimes.entry(runtime.id.clone()).or_default();
            let changes = if asleep {
                ensure!(
                    runtime.mode == "retired",
                    "sleep inventory requires a retired runtime"
                );
                if cache.sleep_revision.as_deref() == Some(stamp.as_str()) {
                    None
                } else {
                    let value = host
                        .store
                        .definition("runtime_sleep", &runtime.id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("sleep inventory missing"))?;
                    let inventory: Inventory = serde_json::from_value(value)?;
                    cache.validate(host, &runtime.id, &inventory)?;
                    cache.snapshot = Some(inventory.snapshot);
                    Some(InventoryChanges {
                        revision: 0,
                        full: true,
                        records: inventory.indexes,
                        removed_ids: Vec::new(),
                    })
                }
            } else {
                if cache.sleep_revision.as_ref().is_some_and(|s| s != &stamp) {
                    cache.revision = None;
                    cache.generation = None;
                }
                Some(cache.fetch(host, runtime, reconcile).await?)
            };
            if let Some(changes) = changes {
                ensure!(
                    changes
                        .records
                        .iter()
                        .all(|record| record.node_id == host.registration.id),
                    "inventory changed node ownership"
                );
                let ids: Vec<_> = changes.records.iter().map(|r| r.id.as_str()).collect();
                host.store
                    .assign_runtime_inventory(&runtime.id, &ids)
                    .await?;
                let mut next_ids = if changes.full {
                    BTreeSet::new()
                } else {
                    std::mem::take(&mut cache.ids)
                };
                for record in changes.records {
                    next_ids.insert(record.id.clone());
                    self.log.set(record);
                }
                if changes.full {
                    for id in cache.ids.difference(&next_ids) {
                        self.log.remove(id);
                    }
                }
                for id in changes.removed_ids {
                    next_ids.remove(&id);
                    self.log.remove(&id);
                }
                cache.ids = next_ids;
                cache.revision = Some(changes.revision);
            }
            cache.sleep_revision = Some(stamp);
            let snapshot = cache
                .snapshot
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot missing"))?;
            active_loops += snapshot.active_agent_loops;
            if runtime.mode == "active" {
                ready = true;
                runtime_ready &= snapshot.ready;
                if let Some(error) = &snapshot.resource_error {
                    errors.push(format!("{}: {error}", runtime.id));
                }
            }
        }
        let capacity = host.store.capacity().await?;
        let removed: Vec<_> = self
            .runtimes
            .keys()
            .filter(|id| !current.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            if let Some(runtime) = self.runtimes.remove(&id) {
                for id in runtime.ids {
                    self.log.remove(&id);
                }
            }
        }
        let mut snapshot = host.snapshot.write().unwrap();
        snapshot.active_agent_loops = active_loops;
        snapshot.active_runs = capacity.running;
        snapshot.pending_runs = capacity.queued;
        snapshot.max_runs = capacity.max_runs;
        snapshot.ready = ready && runtime_ready;
        snapshot.resource_error = (!errors.is_empty()).then(|| errors.join("; "));
        if reconcile {
            self.checked = Some(Instant::now());
        }
        Ok(())
    }
}
