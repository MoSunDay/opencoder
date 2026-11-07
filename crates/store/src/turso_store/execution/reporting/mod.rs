mod queue;
mod read;
use anyhow::Result;
pub(super) use queue::{acknowledge, recover, update};
pub(super) use read::{acks, has_pending, operation, pending, state};
use turso::Connection;

pub(super) async fn initialize(conn: &Connection) -> Result<()> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS execution_report_acks(execution_id TEXT NOT NULL,action TEXT NOT NULL,item TEXT NOT NULL,cursor INTEGER NOT NULL,PRIMARY KEY(execution_id,action,item))",
        "CREATE TABLE IF NOT EXISTS execution_report_queue(key TEXT PRIMARY KEY,execution_id TEXT NOT NULL,action TEXT NOT NULL,item TEXT NOT NULL,cursor INTEGER NOT NULL,payload TEXT NOT NULL,eligible INTEGER NOT NULL DEFAULT 1,UNIQUE(execution_id,action,item))",
        "CREATE TABLE IF NOT EXISTS execution_report_runs(run_id TEXT PRIMARY KEY,generation INTEGER NOT NULL,last_event_seq INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS execution_report_guidance ON execution_report_queue(execution_id,action,cursor)",
        "CREATE TABLE IF NOT EXISTS execution_report_format(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL)",
    ] {conn.execute(sql,()).await?;}
    let mut columns = conn
        .query("PRAGMA table_info(execution_report_queue)", ())
        .await?;
    let mut has_eligible = false;
    while let Some(row) = columns.next().await? {
        has_eligible |= row.get::<String>(1)? == "eligible";
    }
    drop(columns);
    if !has_eligible {
        // Bootstrap already owns the schema transaction: the column and its
        // backfill must commit together, including after an interrupted upgrade.
        conn.execute(
            "ALTER TABLE execution_report_queue ADD COLUMN eligible INTEGER NOT NULL DEFAULT 1",
            (),
        )
        .await?;
        conn.execute("UPDATE execution_report_queue AS q SET eligible=NOT EXISTS(SELECT 1 FROM execution_report_queue earlier WHERE earlier.execution_id=q.execution_id AND earlier.action='layered_guidance' AND earlier.cursor<q.cursor) WHERE q.action='layered_guidance'", ()).await?;
    }
    conn.execute("CREATE INDEX IF NOT EXISTS execution_reports_ready ON execution_report_queue(key) WHERE eligible=1", ()).await?;
    Ok(())
}

pub(super) async fn format(conn: &Connection, reader: u32, writer: u32) -> Result<()> {
    anyhow::ensure!(writer <= reader, "report writer exceeds reader");
    let mut rows = conn
        .query("SELECT version FROM execution_report_format WHERE id=1", ())
        .await?;
    let stored = match rows.next().await? {
        Some(row) => row.get::<i64>(0)?,
        None => 0,
    };
    anyhow::ensure!(
        stored <= reader as i64,
        "report state requires a newer Runtime reader"
    );
    drop(rows);
    if writer as i64 > stored {
        conn.execute("INSERT INTO execution_report_format(id,version) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET version=excluded.version",[writer as i64]).await?;
    }
    Ok(())
}
