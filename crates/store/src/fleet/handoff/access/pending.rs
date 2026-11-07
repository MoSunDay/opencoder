use super::FleetStore;
use anyhow::Result;
use opencoder_core::fleet::Assignment;

impl FleetStore {
    pub async fn prepared_receipt_for(&self, scope: &str, subject: &str) -> Result<Option<String>> {
        let scope = scope.to_owned();
        let subject = subject.to_owned();
        self.db
            .read(move |store| super::prepared_receipt_for(store, &scope, &subject))
            .await
    }
    pub async fn pending_receipts(
        &self,
        scope: &str,
        after: &str,
        limit: u32,
    ) -> Result<Vec<(String, super::Receipt)>> {
        let scope = scope.to_owned();
        let after = after.to_owned();
        self.db
            .read(move |store| super::pending_receipts(store, &scope, &after, limit))
            .await
    }
    pub async fn pending_assignments(&self, after: &str, limit: u32) -> Result<Vec<Assignment>> {
        let after = after.to_owned();
        self.db
            .read(move |store| super::pending_assignments(store, &after, limit))
            .await
    }
}
