//! Publish a node's durable acceptance without replacing newer report status.
use super::super::{records::read_index, FleetStore};
use anyhow::{ensure, Result};
use libsql::{params, Connection, TransactionBehavior};
use opencoder_core::fleet::{Assignment, ExecutionIndex};

pub(super) async fn insert(
    conn: &Connection,
    accepted: &ExecutionIndex,
    receipt_key: Option<&str>,
) -> Result<()> {
    let mut rows = conn
        .query(
            "SELECT assignment FROM execution_assignments WHERE id=?1",
            [accepted.id.as_str()],
        )
        .await?;
    if let Some(row) = rows.next().await? {
        let assignment: Assignment = serde_json::from_str(&row.get::<String>(0)?)?;
        validate_owner(&assignment.index, accepted)?;
        if let Some(key) = receipt_key {
            ensure!(
                super::receipts::dispatch_key(&assignment.request) == key,
                "acceptance does not match dispatch receipt"
            );
        }
    } else {
        ensure!(receipt_key.is_none(), "accepted assignment missing");
    }
    drop(rows);
    if let Some(current) = read_index(conn, &accepted.id).await? {
        validate_owner(&current, accepted)?;
    }
    conn.execute(
        "INSERT INTO execution_index(id,created_at,kind,node_id,status) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO NOTHING",
        params![accepted.id.clone(), accepted.created_at, accepted.kind.prefix(), accepted.node_id.clone(), accepted.status.as_str()],
    ).await?;
    Ok(())
}

fn validate_owner(expected: &ExecutionIndex, actual: &ExecutionIndex) -> Result<()> {
    ensure!(
        expected.id == actual.id
            && expected.kind == actual.kind
            && expected.node_id == actual.node_id
            && expected.created_at == actual.created_at,
        "execution ownership conflict: {}",
        actual.id
    );
    Ok(())
}

impl FleetStore {
    /// Both timely and late Create replies can confirm an execution. Node
    /// reports remain the authority for subsequent status changes.
    pub async fn accept_execution(&self, accepted: &ExecutionIndex) -> Result<()> {
        let _guard = self.gate.lock().await;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        insert(&tx, accepted, None).await?;
        tx.commit().await?;
        Ok(())
    }
}
