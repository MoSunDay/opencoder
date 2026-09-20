use super::super::FleetStore;
use anyhow::{ensure, Context, Result};
use libsql::{params, TransactionBehavior};
use serde::Serialize;

// The optimistic read and transactional update must use the same FIFO fence.
const CLAIMABLE: &str = "ticket=?1 AND runtime_id=?2 AND phase='queued'
    AND sequence=(SELECT min(sequence) FROM capacity_queue WHERE phase='queued')
    AND (SELECT count(*) FROM capacity_queue WHERE phase='running')
        < (SELECT max_runs FROM host_capacity WHERE singleton=1)";

#[derive(Debug, Clone, Serialize)]
pub struct CapacitySnapshot {
    pub max_runs: u64,
    pub running: u64,
    pub queued: u64,
}

impl FleetStore {
    pub async fn initialize_capacity(&self, max_runs: usize) -> Result<()> {
        ensure!(
            max_runs > 0 && max_runs <= i64::MAX as usize,
            "invalid host capacity"
        );
        let _gate = self.gate.lock().await;
        self.conn
            .execute(
                "INSERT INTO host_capacity VALUES (1,?1) ON CONFLICT(singleton) DO NOTHING",
                [max_runs as i64],
            )
            .await?;
        Ok(())
    }
    pub async fn configure_capacity(&self, max_runs: usize) -> Result<()> {
        ensure!(
            max_runs > 0 && max_runs <= i64::MAX as usize,
            "invalid host capacity"
        );
        let _gate = self.gate.lock().await;
        self.conn.execute("INSERT INTO host_capacity VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET max_runs=excluded.max_runs", [max_runs as i64]).await?;
        Ok(())
    }

    pub async fn capacity(&self) -> Result<CapacitySnapshot> {
        let _gate = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT max_runs,(SELECT count(*) FROM capacity_queue WHERE phase='running'),(SELECT count(*) FROM capacity_queue WHERE phase='queued') FROM host_capacity WHERE singleton=1", ()).await?;
        let row = rows
            .next()
            .await?
            .context("host capacity is not configured")?;
        Ok(CapacitySnapshot {
            max_runs: row.get::<i64>(0)? as u64,
            running: row.get::<i64>(1)? as u64,
            queued: row.get::<i64>(2)? as u64,
        })
    }

    pub async fn enqueue_capacity(
        &self,
        ticket: &str,
        execution: &str,
        runtime: &str,
    ) -> Result<i64> {
        let _gate = self.gate.lock().await;
        // Schedulers replay accepted tickets on every tick. An ignored INSERT
        // still writes SQLite's AUTOINCREMENT counter and can starve admission.
        if let Some(sequence) = self
            .capacity_ticket_sequence(ticket, execution, runtime)
            .await?
        {
            return Ok(sequence);
        }
        self.conn.execute("INSERT INTO capacity_queue(ticket,execution_id,runtime_id,phase) VALUES (?1,?2,?3,'queued') ON CONFLICT(ticket) DO NOTHING", params![ticket,execution,runtime]).await?;
        self.capacity_ticket_sequence(ticket, execution, runtime)
            .await?
            .context("capacity ticket disappeared after enqueue")
    }

    /// Caller holds this connection's gate; other connections may still race
    /// the insertion, so ownership is checked on both reads.
    async fn capacity_ticket_sequence(
        &self,
        ticket: &str,
        execution: &str,
        runtime: &str,
    ) -> Result<Option<i64>> {
        let mut rows = self
            .conn
            .query(
                "SELECT sequence,execution_id,runtime_id,phase FROM capacity_queue WHERE ticket=?1",
                [ticket],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        ensure!(
            row.get::<String>(1)? == execution
                && row.get::<String>(2)? == runtime
                && row.get::<String>(3)? != "done",
            "capacity ticket conflict"
        );
        Ok(Some(row.get(0)?))
    }

    /// Strict machine-wide FIFO, including retired runtimes. Heartbeat age is
    /// deliberately irrelevant: a missing host must never free live slots.
    pub async fn claim_capacity(&self, ticket: &str, runtime: &str) -> Result<bool> {
        let _gate = self.gate.lock().await;
        let eligible = {
            let mut rows = self
                .conn
                .query(
                    &format!("SELECT EXISTS(SELECT 1 FROM capacity_queue WHERE {CLAIMABLE})"),
                    params![ticket, runtime],
                )
                .await?;
            rows.next()
                .await?
                .context("capacity eligibility missing")?
                .get::<i64>(0)?
                != 0
        };
        if !eligible {
            return Ok(false);
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let changed = tx
            .execute(
                &format!("UPDATE capacity_queue SET phase='running' WHERE {CLAIMABLE}"),
                params![ticket, runtime],
            )
            .await?;
        tx.commit().await?;
        Ok(changed == 1)
    }

    /// Only the owning runtime calls this after completion and durable writes.
    /// No timer, host retirement, or release rollback is allowed to call it.
    pub async fn finish_capacity(&self, ticket: &str, runtime: &str) -> Result<()> {
        let _gate = self.gate.lock().await;
        let changed = self
            .conn
            .execute(
                "UPDATE capacity_queue SET phase='done' WHERE ticket=?1 AND runtime_id=?2",
                params![ticket, runtime],
            )
            .await?;
        ensure!(changed == 1, "unknown capacity owner");
        Ok(())
    }

    pub async fn runtime_tickets(&self, runtime: &str) -> Result<Vec<(String, String, String)>> {
        let _gate = self.gate.lock().await;
        let mut rows = self.conn.query("SELECT ticket,execution_id,phase FROM capacity_queue WHERE runtime_id=?1 AND phase!='done' ORDER BY sequence", [runtime]).await?;
        let mut tickets = Vec::new();
        while let Some(row) = rows.next().await? {
            tickets.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
        Ok(tickets)
    }
}
