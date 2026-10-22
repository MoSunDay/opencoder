use super::*;
use crate::host::config::{InventoryUpdate, RuntimeConfig};
use opencoder_store::fleet::handoff::RuntimeRecord;

impl RuntimeInventory {
    pub(super) fn validate(&self, host: &Host, id: &str, inventory: &Inventory) -> Result<()> {
        ensure!(
            inventory.runtime_id.as_deref() == Some(id),
            "runtime endpoint returned another owner"
        );
        ensure!(
            inventory.registration.id == host.registration.id,
            "runtime changed node identity"
        );
        ensure!(
            inventory.registration.protocol_version == host.registration.protocol_version,
            "runtime protocol is incompatible"
        );
        Ok(())
    }
    pub(super) async fn fetch(
        &mut self,
        host: &Host,
        runtime: &RuntimeRecord,
        full: bool,
    ) -> Result<InventoryChanges> {
        if self.legacy {
            let inventory = host.inventory(runtime, false).await?;
            self.snapshot = Some(inventory.snapshot);
            return Ok(InventoryChanges {
                revision: 0,
                full: true,
                records: inventory.indexes,
                removed_ids: Vec::new(),
            });
        }
        let config: RuntimeConfig = serde_json::from_value(runtime.config.clone())?;
        config.validate()?;
        let mut query = InventoryQuery {
            generation: self.generation.clone(),
            after: if full { None } else { self.revision },
            page: None,
        };
        let mut collected: Option<InventoryChanges> = None;
        let mut generation = None;
        let mut final_snapshot;
        let mut seen = BTreeSet::new();
        let mut page_tokens = BTreeSet::new();
        loop {
            let response = host
                .client
                .get(format!(
                    "{}/inventory/changes",
                    config.endpoint.trim_end_matches('/')
                ))
                .bearer_auth(&host.token)
                .query(&query)
                .timeout(Duration::from_secs(15))
                .send()
                .await?;
            if query.page.is_none() && matches!(response.status().as_u16(), 404 | 405) {
                let inventory = host.inventory(runtime, false).await?;
                self.legacy = true;
                self.snapshot = Some(inventory.snapshot);
                return Ok(InventoryChanges {
                    revision: 0,
                    full: true,
                    records: inventory.indexes,
                    removed_ids: Vec::new(),
                });
            }
            let update: InventoryUpdate = response.error_for_status()?.json().await?;
            self.validate(host, &runtime.id, &update.inventory)?;
            let page = update.changes;
            ensure!(
                update.inventory.snapshot.generation == page.generation,
                "inventory snapshot generation mismatch"
            );
            ensure!(
                !page.full || page.removed_ids.is_empty(),
                "full inventory cannot remove IDs"
            );
            if query.page.is_none() {
                ensure!(
                    query.after.is_some() || page.full,
                    "initial inventory must be full"
                );
                ensure!(
                    page.full || query.generation.as_deref() == Some(&page.generation),
                    "delta inventory generation mismatch"
                );
                ensure!(
                    page.full || query.after.is_none_or(|after| page.revision >= after),
                    "inventory cursor regressed"
                );
            }
            ensure!(
                page.records.len() + page.removed_ids.len() <= 256,
                "runtime inventory page exceeds limit"
            );
            if let Some(expected) = &generation {
                ensure!(
                    expected == &page.generation,
                    "runtime inventory generation changed during paging"
                );
            } else {
                generation = Some(page.generation.clone());
            }
            let changes = collected.get_or_insert_with(|| InventoryChanges {
                revision: page.revision,
                full: page.full,
                records: Vec::new(),
                removed_ids: Vec::new(),
            });
            ensure!(
                changes.revision == page.revision && changes.full == page.full,
                "runtime inventory snapshot changed during paging"
            );
            for record in &page.records {
                ensure!(
                    seen.insert(record.id.clone()),
                    "duplicate runtime inventory identity"
                );
            }
            for id in &page.removed_ids {
                ensure!(
                    seen.insert(id.clone()),
                    "duplicate runtime inventory removal"
                );
            }
            changes.records.extend(page.records);
            changes.removed_ids.extend(page.removed_ids);
            final_snapshot = update.inventory.snapshot;
            query.page = page.next;
            if query.page.is_none() {
                break;
            }
            ensure!(
                page_tokens.insert(query.page.clone().expect("next page")),
                "inventory page cursor repeated"
            );
        }
        self.generation = generation;
        self.snapshot = Some(final_snapshot);
        Ok(collected.expect("first inventory page"))
    }
}
