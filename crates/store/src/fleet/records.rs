use super::FleetStore;
use anyhow::{bail, Context, Result};
use libsql::params;
use opencoder_core::fleet::{
    ExecutionCursor, ExecutionIndex, ExecutionKind, ExecutionPage, NodeRegistration,
    EXECUTION_PAGE_MAX,
};
use serde_json::Value;

impl FleetStore {
    pub async fn register(&self, node: &NodeRegistration) -> Result<()> {
        let _guard = self.gate.lock().await;
        self.conn.execute("INSERT INTO fleet_nodes VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET registration=excluded.registration",
            params![node.id.clone(), serde_json::to_string(node)?]).await?;
        Ok(())
    }
    /// Remove only the node registration; execution ownership and history
    /// remain in execution_index and in the node's own durable store.
    pub async fn unregister(&self, id: &str) -> Result<bool> {
        let _guard = self.gate.lock().await;
        Ok(self
            .conn
            .execute("DELETE FROM fleet_nodes WHERE id=?1", params![id])
            .await?
            > 0)
    }
    pub async fn nodes(&self) -> Result<Vec<NodeRegistration>> {
        let _guard = self.gate.lock().await;
        let mut rows = self
            .conn
            .query("SELECT registration FROM fleet_nodes ORDER BY id", ())
            .await?;
        let mut result = vec![];
        while let Some(row) = rows.next().await? {
            result.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(result)
    }
    /// Ownership and creation time are immutable, including after reconnect.
    pub async fn put_index(&self, record: &ExecutionIndex) -> Result<()> {
        let _guard = self.gate.lock().await;
        if let Some(old) = read_index(&self.conn, &record.id).await? {
            if old.node_id != record.node_id
                || old.created_at != record.created_at
                || old.kind != record.kind
            {
                bail!("execution ownership conflict: {}", record.id);
            }
        }
        self.conn.execute("INSERT INTO execution_index(id,created_at,kind,node_id,status) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET status=excluded.status",
            params![record.id.clone(), record.created_at, record.kind.prefix(), record.node_id.clone(), record.status.as_str()]).await?;
        Ok(())
    }
    /// Remove execution-index rows by id, ownership-guarded by `node_id` and
    /// `kind`, together with their frozen dispatch assignments and execution
    /// receipts (no foreign keys link these tables, so all three go in one
    /// transaction). Rows whose stored status is NOT terminal are refused so a
    /// stale id list can never drop a live execution. Returns the number of
    /// deleted index rows.
    pub async fn delete_terminal_indexes(
        &self,
        node_id: &str,
        kind: ExecutionKind,
        ids: &[String],
    ) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let _guard = self.gate.lock().await;
        self.conn
            .execute("BEGIN IMMEDIATE", ())
            .await
            .context("begin index deletion transaction")?;
        let result: anyhow::Result<u64> = async {
            let mut removed = 0u64;
            for id in ids {
                let affected = self
                    .conn
                    .execute(
                        "DELETE FROM execution_index
                         WHERE id=?1 AND node_id=?2 AND kind=?3
                           AND status IN ('idle','done','error','cancelled')",
                        params![id.as_str(), node_id, kind.prefix()],
                    )
                    .await
                    .context("delete execution index row")?;
                if affected == 0 {
                    continue;
                }
                removed += 1;
                self.conn
                    .execute("DELETE FROM execution_assignments WHERE id=?1", [id.as_str()])
                    .await
                    .context("delete execution assignment")?;
                self.conn
                    .execute(
                        "DELETE FROM dispatch_receipts WHERE scope='execution' AND id=?1",
                        [id.as_str()],
                    )
                    .await
                    .context("delete dispatch receipt")?;
            }
            Ok(removed)
        }
        .await;
        match result {
            Ok(removed) => {
                self.conn.execute("COMMIT", ()).await?;
                Ok(removed)
            }
            Err(error) => {
                crate::fleet::report::rollback(&self.conn).await;
                Err(error)
            }
        }
    }
    pub async fn index(&self, id: &str) -> Result<Option<ExecutionIndex>> {
        let _guard = self.gate.lock().await;
        read_index(&self.conn, id).await
    }
    pub async fn indexes(
        &self,
        node: Option<&str>,
        kind: Option<ExecutionKind>,
        limit: u32,
    ) -> Result<Vec<ExecutionIndex>> {
        let _guard = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT id,created_at,kind,node_id,status FROM execution_index WHERE (?1 IS NULL OR node_id=?1) AND (?2 IS NULL OR kind=?2) ORDER BY created_at DESC,id LIMIT ?3",
            params![node.map(str::to_owned), kind.map(|value| value.prefix().to_owned()), limit.clamp(1, 10000)]).await?;
        let mut result = vec![];
        while let Some(row) = rows.next().await? {
            result.push(decode_index(row)?);
        }
        Ok(result)
    }
    pub async fn indexes_page(
        &self,
        node: Option<&str>,
        kind: Option<ExecutionKind>,
        cursor: Option<&ExecutionCursor>,
        limit: u32,
    ) -> Result<ExecutionPage<ExecutionIndex>> {
        let _guard = self.gate.lock().await;
        let limit = limit.clamp(1, EXECUTION_PAGE_MAX);
        let mut rows = self
            .conn
            .query(
                "SELECT id,created_at,kind,node_id,status FROM execution_index \
             WHERE (?1 IS NULL OR node_id=?1) AND (?2 IS NULL OR kind=?2) \
             AND (?3 IS NULL OR created_at<?3 OR (created_at=?3 AND id>?4)) \
             ORDER BY created_at DESC,id ASC LIMIT ?5",
                params![
                    node.map(str::to_owned),
                    kind.map(|value| value.prefix().to_owned()),
                    cursor.map(|value| value.created_at),
                    cursor.map(|value| value.id.clone()),
                    limit as i64 + 1,
                ],
            )
            .await?;
        let mut executions = Vec::with_capacity(limit as usize + 1);
        while let Some(row) = rows.next().await? {
            executions.push(decode_index(row)?);
        }
        let more = executions.len() > limit as usize;
        executions.truncate(limit as usize);
        let next_cursor = more.then(|| {
            let last = executions.last().expect("non-empty page with extra row");
            ExecutionCursor {
                created_at: last.created_at,
                id: last.id.clone(),
            }
        });
        Ok(ExecutionPage {
            executions,
            next_cursor,
        })
    }
    pub async fn definition(&self, kind: &str, id: &str) -> Result<Option<Value>> {
        let _guard = self.gate.lock().await;
        self.definition_locked(kind, id).await
    }
    pub(super) async fn definition_locked(&self, kind: &str, id: &str) -> Result<Option<Value>> {
        let mut rows = self
            .conn
            .query(
                "SELECT body FROM fleet_definitions WHERE kind=?1 AND id=?2",
                params![kind, id],
            )
            .await?;
        rows.next()
            .await?
            .map(|r| Ok(serde_json::from_str(&r.get::<String>(0)?)?))
            .transpose()
    }
    pub async fn definitions(&self, kind: &str) -> Result<Vec<Value>> {
        let _guard = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT body FROM fleet_definitions WHERE kind=?1 ORDER BY id",
                [kind],
            )
            .await?;
        let mut result = vec![];
        while let Some(row) = rows.next().await? {
            result.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(result)
    }
    pub async fn delete_definition(&self, kind: &str, id: &str) -> Result<()> {
        let _guard = self.gate.lock().await;
        self.conn
            .execute(
                "DELETE FROM fleet_definitions WHERE kind=?1 AND id=?2",
                params![kind, id],
            )
            .await?;
        Ok(())
    }
    pub async fn put_definition(&self, kind: &str, id: &str, body: &Value) -> Result<()> {
        let _guard = self.gate.lock().await;
        self.put_definition_locked(kind, id, body).await
    }
    pub(super) async fn put_definition_locked(
        &self,
        kind: &str,
        id: &str,
        body: &Value,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO fleet_definitions VALUES (?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET body=excluded.body", params![kind,id,serde_json::to_string(body)?]).await?;
        Ok(())
    }
}

pub(super) async fn read_index(
    conn: &libsql::Connection,
    id: &str,
) -> Result<Option<ExecutionIndex>> {
    let mut rows = conn
        .query(
            "SELECT id,created_at,kind,node_id,status FROM execution_index WHERE id=?1",
            [id],
        )
        .await?;
    rows.next().await?.map(decode_index).transpose()
}

pub(super) fn decode_index(row: libsql::Row) -> Result<ExecutionIndex> {
    Ok(ExecutionIndex {
        id: row.get(0)?,
        created_at: row.get(1)?,
        kind: serde_json::from_value(Value::String(row.get(2)?))?,
        node_id: row.get(3)?,
        status: serde_json::from_value(Value::String(row.get(4)?))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::ExecutionStatus;
    #[tokio::test]
    async fn immutable_owner_kind_and_creation_time_with_definition_separation() {
        let store = FleetStore::open_memory().await.unwrap();
        let mut rec = ExecutionIndex {
            id: "agent-x".into(),
            created_at: 1,
            kind: ExecutionKind::Agent,
            node_id: "n1".into(),
            status: ExecutionStatus::Pending,
        };
        store.put_index(&rec).await.unwrap();
        rec.status = ExecutionStatus::Running;
        store.put_index(&rec).await.unwrap();
        rec.node_id = "n2".into();
        assert!(store.put_index(&rec).await.is_err());
        rec.node_id = "n1".into();
        rec.kind = ExecutionKind::Team;
        assert!(store.put_index(&rec).await.is_err());
        rec.kind = ExecutionKind::Agent;
        rec.created_at = 2;
        assert!(store.put_index(&rec).await.is_err());
        assert_eq!(store.index("agent-x").await.unwrap().unwrap().node_id, "n1");
        assert!(store.index("missing").await.unwrap().is_none());
        let mut rows = store
            .conn
            .query("PRAGMA table_info(execution_index)", ())
            .await
            .unwrap();
        let mut columns = vec![];
        while let Some(row) = rows.next().await.unwrap() {
            columns.push(row.get::<String>(1).unwrap());
        }
        assert_eq!(columns, ["id", "created_at", "kind", "node_id", "status"]);
        store
            .put_definition("team", "a", &serde_json::json!({"name":"a"}))
            .await
            .unwrap();
        assert_eq!(store.definitions("team").await.unwrap().len(), 1);
        assert_eq!(
            store
                .indexes(None, Some(ExecutionKind::Agent), 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
