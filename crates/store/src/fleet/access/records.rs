use super::*;

impl FleetStore {
    pub async fn register(&self, node: &NodeRegistration) -> Result<()> {
        let node = node.to_owned();
        self.db
            .write(move |conn| async move {
                let node = &node;
                FleetConnection { conn }.register(node).await
            })
            .await
    }
    pub async fn unregister(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let id = &id;
                FleetConnection { conn }.unregister(id).await
            })
            .await
    }
    pub async fn nodes(&self) -> Result<Vec<NodeRegistration>> {
        self.db
            .read(move |conn| async move { FleetConnection { conn }.nodes().await })
            .await
    }
    pub async fn put_index(&self, record: &ExecutionIndex) -> Result<()> {
        let record = record.to_owned();
        self.db
            .write(move |conn| async move {
                let record = &record;
                FleetConnection { conn }.put_index(record).await
            })
            .await
    }
    pub async fn delete_terminal_indexes(
        &self,
        node_id: &str,
        kind: ExecutionKind,
        ids: &[String],
    ) -> Result<u64> {
        let node_id = node_id.to_owned();
        let ids = ids.to_owned();
        self.db
            .write(move |conn| async move {
                let node_id = &node_id;
                let ids = &ids;
                FleetConnection { conn }
                    .delete_terminal_indexes(node_id, kind, ids)
                    .await
            })
            .await
    }
    pub async fn index(&self, id: &str) -> Result<Option<ExecutionIndex>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = &id;
                FleetConnection { conn }.index(id).await
            })
            .await
    }
    pub async fn indexes(
        &self,
        node: Option<&str>,
        kind: Option<ExecutionKind>,
        limit: u32,
    ) -> Result<Vec<ExecutionIndex>> {
        let node = node.map(ToOwned::to_owned);
        self.db
            .read(move |conn| async move {
                let node = node.as_deref();
                FleetConnection { conn }.indexes(node, kind, limit).await
            })
            .await
    }
    pub async fn indexes_page(
        &self,
        node: Option<&str>,
        kind: Option<ExecutionKind>,
        cursor: Option<&ExecutionCursor>,
        limit: u32,
    ) -> Result<ExecutionPage<ExecutionIndex>> {
        let node = node.map(ToOwned::to_owned);
        let cursor = cursor.map(ToOwned::to_owned);
        self.db
            .read(move |conn| async move {
                let node = node.as_deref();
                let cursor = cursor.as_ref();
                FleetConnection { conn }
                    .indexes_page(node, kind, cursor, limit)
                    .await
            })
            .await
    }
    pub async fn definition(&self, kind: &str, id: &str) -> Result<Option<Value>> {
        let kind = kind.to_owned();
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let kind = &kind;
                let id = &id;
                FleetConnection { conn }.definition(kind, id).await
            })
            .await
    }
    pub async fn definitions(&self, kind: &str) -> Result<Vec<Value>> {
        let kind = kind.to_owned();
        self.db
            .read(move |conn| async move {
                let kind = &kind;
                FleetConnection { conn }.definitions(kind).await
            })
            .await
    }
    pub async fn delete_definition(&self, kind: &str, id: &str) -> Result<()> {
        let kind = kind.to_owned();
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let kind = &kind;
                let id = &id;
                FleetConnection { conn }.delete_definition(kind, id).await
            })
            .await
    }
    pub async fn put_definition(&self, kind: &str, id: &str, body: &Value) -> Result<()> {
        let kind = kind.to_owned();
        let id = id.to_owned();
        let body = body.to_owned();
        self.db
            .write(move |conn| async move {
                let kind = &kind;
                let id = &id;
                let body = &body;
                FleetConnection { conn }
                    .put_definition(kind, id, body)
                    .await
            })
            .await
    }
}
