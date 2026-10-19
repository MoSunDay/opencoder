use super::*;

impl FleetStore {
    pub async fn receipt(&self, scope: &str, id: &str) -> Result<Option<Receipt>> {
        let scope = scope.to_owned();
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let scope = &scope;
                let id = &id;
                FleetConnection { conn }.receipt(scope, id).await
            })
            .await
    }
    pub async fn claim_request(&self, scope: &str, id: &str, fingerprint: &str) -> Result<bool> {
        let scope = scope.to_owned();
        let id = id.to_owned();
        let fingerprint = fingerprint.to_owned();
        self.db
            .write(move |conn| async move {
                let scope = &scope;
                let id = &id;
                let fingerprint = &fingerprint;
                FleetConnection { conn }
                    .claim_request(scope, id, fingerprint)
                    .await
            })
            .await
    }
    pub async fn save_receipt(&self, scope: &str, id: &str, receipt: &Receipt) -> Result<()> {
        let scope = scope.to_owned();
        let id = id.to_owned();
        let receipt = receipt.to_owned();
        self.db
            .write(move |conn| async move {
                let scope = &scope;
                let id = &id;
                let receipt = &receipt;
                FleetConnection { conn }
                    .save_receipt(scope, id, receipt)
                    .await
            })
            .await
    }
    pub async fn assignment(&self, id: &str) -> Result<Option<Assignment>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = &id;
                FleetConnection { conn }.assignment(id).await
            })
            .await
    }
    pub async fn execution_names(&self, ids: &[String]) -> Result<HashMap<String, ExecutionNames>> {
        let ids = ids.to_owned();
        self.db
            .read(move |conn| async move {
                let ids = &ids;
                FleetConnection { conn }.execution_names(ids).await
            })
            .await
    }
    pub async fn prepare_assignment(
        &self,
        assignment: &Assignment,
        fingerprint: &str,
    ) -> Result<()> {
        let assignment = assignment.to_owned();
        let fingerprint = fingerprint.to_owned();
        self.db
            .write(move |conn| async move {
                let assignment = &assignment;
                let fingerprint = &fingerprint;
                FleetConnection { conn }
                    .prepare_assignment(assignment, fingerprint)
                    .await
            })
            .await
    }
    pub async fn finish_dispatch(
        &self,
        id: &str,
        fingerprint: &str,
        reply: &RpcReply,
    ) -> Result<()> {
        let id = id.to_owned();
        let fingerprint = fingerprint.to_owned();
        let reply = reply.to_owned();
        self.db
            .write(move |conn| async move {
                let id = &id;
                let fingerprint = &fingerprint;
                let reply = &reply;
                FleetConnection { conn }
                    .finish_dispatch(id, fingerprint, reply)
                    .await
            })
            .await
    }
}
