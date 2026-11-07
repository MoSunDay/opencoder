use super::FleetStore;
use anyhow::Result;
use opencoder_core::fleet::{
    ExecutionCursor, ExecutionIndex, ExecutionKind, ExecutionPage, NodeRegistration,
};
use serde_json::Value;

impl FleetStore {
    pub async fn register(&self, node: &NodeRegistration) -> Result<()> {
        let node = node.to_owned();
        self.db
            .write(move |store| super::register(store, &node))
            .await
    }
    pub async fn unregister(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db
            .write(move |store| super::unregister(store, &id))
            .await
    }
    pub async fn nodes(&self) -> Result<Vec<NodeRegistration>> {
        self.db.read(super::nodes).await
    }
    pub async fn put_index(&self, record: &ExecutionIndex) -> Result<()> {
        let record = record.to_owned();
        self.db
            .write(move |store| super::put_index(store, &record))
            .await
    }
    pub async fn delete_terminal_indexes(
        &self,
        node_id: &str,
        kind: ExecutionKind,
        ids: &[String],
    ) -> Result<u64> {
        let node_id = node_id.to_owned();
        let ids = ids.to_vec();
        self.db
            .write(move |store| super::delete_terminal_indexes(store, &node_id, kind, &ids))
            .await
    }
    pub async fn index(&self, id: &str) -> Result<Option<ExecutionIndex>> {
        let id = id.to_owned();
        self.db.read(move |store| super::index(store, &id)).await
    }
    pub async fn indexes(
        &self,
        node: Option<&str>,
        kind: Option<ExecutionKind>,
        limit: u32,
    ) -> Result<Vec<ExecutionIndex>> {
        let node = node.map(str::to_owned);
        self.db
            .read(move |store| super::indexes(store, node.as_deref(), kind, limit))
            .await
    }
    pub async fn indexes_page(
        &self,
        node: Option<&str>,
        kind: Option<ExecutionKind>,
        cursor: Option<&ExecutionCursor>,
        limit: u32,
    ) -> Result<ExecutionPage<ExecutionIndex>> {
        let node = node.map(str::to_owned);
        let cursor = cursor.cloned();
        self.db
            .read(move |store| {
                super::indexes_page(store, node.as_deref(), kind, cursor.as_ref(), limit)
            })
            .await
    }
    pub async fn definition(&self, kind: &str, id: &str) -> Result<Option<Value>> {
        let kind = kind.to_owned();
        let id = id.to_owned();
        self.db
            .read(move |store| super::definition(store, &kind, &id))
            .await
    }
    pub async fn definitions(&self, kind: &str) -> Result<Vec<Value>> {
        let kind = kind.to_owned();
        self.db
            .read(move |store| super::definitions(store, &kind))
            .await
    }
    pub async fn delete_definition(&self, kind: &str, id: &str) -> Result<()> {
        let kind = kind.to_owned();
        let id = id.to_owned();
        self.db
            .write(move |store| super::delete_definition(store, &kind, &id))
            .await
    }
    pub async fn put_definition(&self, kind: &str, id: &str, body: &Value) -> Result<()> {
        let kind = kind.to_owned();
        let id = id.to_owned();
        let body = body.clone();
        self.db
            .write(move |store| super::put_definition(store, &kind, &id, &body))
            .await
    }
}
