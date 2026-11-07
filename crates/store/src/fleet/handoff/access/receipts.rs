use super::FleetStore;
use anyhow::Result;
use opencoder_core::fleet::{Assignment, RpcReply};
use std::collections::HashMap;

impl FleetStore {
    pub async fn receipt(&self, scope: &str, id: &str) -> Result<Option<super::Receipt>> {
        let scope = scope.to_owned();
        let id = id.to_owned();
        self.db
            .read(move |store| super::receipt(store, &scope, &id))
            .await
    }
    pub async fn claim_request(&self, scope: &str, id: &str, fingerprint: &str) -> Result<bool> {
        let scope = scope.to_owned();
        let id = id.to_owned();
        let fingerprint = fingerprint.to_owned();
        self.db
            .write(move |store| super::claim_request(store, &scope, &id, &fingerprint))
            .await
    }
    pub async fn save_receipt(
        &self,
        scope: &str,
        id: &str,
        receipt: &super::Receipt,
    ) -> Result<()> {
        let scope = scope.to_owned();
        let id = id.to_owned();
        let receipt = receipt.to_owned();
        self.db
            .write(move |store| super::save_receipt(store, &scope, &id, &receipt))
            .await
    }
    pub async fn assignment(&self, id: &str) -> Result<Option<Assignment>> {
        let id = id.to_owned();
        self.db
            .read(move |store| super::assignment(store, &id))
            .await
    }
    pub async fn execution_names(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, super::ExecutionNames>> {
        let ids = ids.to_vec();
        self.db
            .read(move |store| super::execution_names(store, &ids))
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
            .write(move |store| super::prepare_assignment(store, &assignment, &fingerprint))
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
            .write(move |store| super::finish_dispatch(store, &id, &fingerprint, &reply))
            .await
    }
}
