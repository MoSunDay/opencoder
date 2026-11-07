use super::*;

impl FleetStore {
    pub async fn prepared_receipt_for(&self, scope: &str, subject: &str) -> Result<Option<String>> {
        let scope = scope.to_owned();
        let subject = subject.to_owned();
        self.db
            .read(move |conn| async move {
                let scope = &scope;
                let subject = &subject;
                FleetConnection { conn }
                    .prepared_receipt_for(scope, subject)
                    .await
            })
            .await
    }
    pub async fn pending_receipts(
        &self,
        scope: &str,
        after: &str,
        limit: u32,
    ) -> Result<Vec<(String, crate::fleet::handoff::Receipt)>> {
        let scope = scope.to_owned();
        let after = after.to_owned();
        self.db
            .read(move |conn| async move {
                let scope = &scope;
                let after = &after;
                FleetConnection { conn }
                    .pending_receipts(scope, after, limit)
                    .await
            })
            .await
    }
    pub async fn pending_assignments(&self, after: &str, limit: u32) -> Result<Vec<Assignment>> {
        let after = after.to_owned();
        self.db
            .read(move |conn| async move {
                let after = &after;
                FleetConnection { conn }
                    .pending_assignments(after, limit)
                    .await
            })
            .await
    }
}
