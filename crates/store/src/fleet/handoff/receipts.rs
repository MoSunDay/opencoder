use super::super::FleetStore;
use anyhow::{ensure, Result};
use libsql::{params, TransactionBehavior};
use opencoder_core::fleet::{Assignment, CreateExecution, ExecutionKind, RpcReply};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Project sessions retain one owner while each initial admission has its own run ID.
pub fn dispatch_key(request: &CreateExecution) -> &str {
    if request.kind == ExecutionKind::Project {
        request.input["run_id"].as_str().unwrap_or(&request.id)
    } else {
        &request.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub fingerprint: String,
    pub phase: String,
    pub payload: Value,
}

impl FleetStore {
    pub async fn pending_assignments(&self, after: &str, limit: u32) -> Result<Vec<Assignment>> {
        let _gate = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT a.assignment FROM execution_assignments a JOIN dispatch_receipts r ON r.scope='execution' AND r.id=CASE WHEN json_extract(a.assignment,'$.request.kind')='project' THEN COALESCE(json_extract(a.assignment,'$.request.input.run_id'),a.id) ELSE a.id END WHERE r.phase='prepared' AND a.id>?1 ORDER BY a.id LIMIT ?2", params![after,i64::from(limit.min(128))]).await?;
        let mut assignments = Vec::new();
        while let Some(row) = rows.next().await? {
            assignments.push(serde_json::from_str(&row.get::<String>(0)?)?);
        }
        Ok(assignments)
    }
    pub async fn receipt(&self, scope: &str, id: &str) -> Result<Option<Receipt>> {
        let _gate = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT fingerprint,phase,payload FROM dispatch_receipts WHERE scope=?1 AND id=?2",
                params![scope, id],
            )
            .await?;
        match rows.next().await? {
            None => Ok(None),
            Some(row) => Ok(Some(Receipt {
                fingerprint: row.get(0)?,
                phase: row.get(1)?,
                payload: serde_json::from_str(&row.get::<String>(2)?)?,
            })),
        }
    }

    /// First writer owns the original intent. Returns false on content reuse.
    pub async fn claim_request(&self, scope: &str, id: &str, fingerprint: &str) -> Result<bool> {
        let _gate = self.gate.lock().await;
        self.conn.execute(
            "INSERT INTO dispatch_receipts VALUES (?1,?2,?3,'claimed','null') ON CONFLICT(scope,id) DO NOTHING",
            params![scope,id,fingerprint],
        ).await?;
        let mut rows = self
            .conn
            .query(
                "SELECT fingerprint FROM dispatch_receipts WHERE scope=?1 AND id=?2",
                params![scope, id],
            )
            .await?;
        Ok(rows.next().await?.unwrap().get::<String>(0)? == fingerprint)
    }

    /// Caller holds request_lock for this key. Updates cannot change intent.
    pub async fn save_receipt(&self, scope: &str, id: &str, receipt: &Receipt) -> Result<()> {
        let _gate = self.gate.lock().await;
        let changed = self.conn.execute(
            "UPDATE dispatch_receipts SET phase=?4,payload=?5 WHERE scope=?1 AND id=?2 AND fingerprint=?3",
            params![scope,id,receipt.fingerprint.clone(),receipt.phase.clone(),serde_json::to_string(&receipt.payload)?],
        ).await?;
        ensure!(changed == 1, "request receipt ownership conflict");
        Ok(())
    }

    pub async fn assignment(&self, id: &str) -> Result<Option<Assignment>> {
        let _gate = self.gate.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT assignment FROM execution_assignments WHERE id=?1",
                [id],
            )
            .await?;
        rows.next()
            .await?
            .map(|row| Ok(serde_json::from_str(&row.get::<String>(0)?)?))
            .transpose()
    }

    /// Atomic dispatch outbox: ownership, frozen input and dispatch phase exist
    /// before the first RPC. A lost reply never leads to a new assignment.
    pub async fn prepare_assignment(
        &self,
        assignment: &Assignment,
        fingerprint: &str,
    ) -> Result<()> {
        let _gate = self.gate.lock().await;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let i = &assignment.index;
        let mut rows = tx
            .query(
                "SELECT assignment FROM execution_assignments WHERE id=?1",
                [i.id.as_str()],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let old: Assignment = serde_json::from_str(&row.get::<String>(0)?)?;
            ensure!(
                old.index.node_id == i.node_id
                    && old.index.kind == i.kind
                    && old.index.created_at == i.created_at,
                "assignment ownership conflict"
            );
            if old.request == assignment.request {
                return Ok(());
            }
            ensure!(
                old.request.kind == ExecutionKind::Project
                    && dispatch_key(&old.request) != dispatch_key(&assignment.request),
                "assignment conflict"
            );
            let mut rejected = tx
                .query(
                    "SELECT phase FROM dispatch_receipts WHERE scope='execution' AND id=?1",
                    [dispatch_key(&old.request)],
                )
                .await?;
            ensure!(
                rejected
                    .next()
                    .await?
                    .is_some_and(|row| row.get::<String>(0).ok().as_deref() == Some("rejected")),
                "previous project dispatch is unresolved or accepted"
            );
        }
        drop(rows);
        tx.execute("INSERT INTO execution_index(id,created_at,kind,node_id,status) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO NOTHING",
            params![i.id.clone(),i.created_at,i.kind.prefix(),i.node_id.clone(),i.status.as_str()]).await?;
        let mut rows = tx
            .query(
                "SELECT node_id,kind,created_at FROM execution_index WHERE id=?1",
                [i.id.as_str()],
            )
            .await?;
        let row = rows.next().await?.unwrap();
        ensure!(
            row.get::<String>(0)? == i.node_id
                && row.get::<String>(1)? == i.kind.prefix()
                && row.get::<i64>(2)? == i.created_at,
            "execution ownership conflict"
        );
        drop(rows);
        tx.execute(
            "INSERT INTO execution_assignments VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET assignment=excluded.assignment",
            params![i.id.clone(), serde_json::to_string(assignment)?],
        )
        .await?;
        let changed = tx.execute("UPDATE dispatch_receipts SET phase='prepared' WHERE scope='execution' AND id=?1 AND fingerprint=?2", params![dispatch_key(&assignment.request),fingerprint]).await?;
        ensure!(changed == 1, "missing dispatch receipt");
        tx.commit().await?;
        Ok(())
    }

    pub async fn finish_dispatch(
        &self,
        id: &str,
        fingerprint: &str,
        reply: &RpcReply,
    ) -> Result<()> {
        self.save_receipt(
            "execution",
            id,
            &Receipt {
                fingerprint: fingerprint.into(),
                phase: "accepted".into(),
                payload: serde_json::to_value(reply)?,
            },
        )
        .await
    }
}
