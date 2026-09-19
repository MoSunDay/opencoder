use anyhow::Result;
use libsql::Connection;
pub(crate) async fn initialize(conn: &Connection) -> Result<()> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS brain_scheduler_runs(run_id TEXT PRIMARY KEY,generation INTEGER NOT NULL,body TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS brain_scheduler_operations(execution_id TEXT PRIMARY KEY,operation_id TEXT NOT NULL UNIQUE,run_id TEXT NOT NULL REFERENCES brain_scheduler_runs(run_id),round INTEGER NOT NULL,body TEXT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS brain_scheduler_operations_round ON brain_scheduler_operations(run_id,round)",
        "CREATE TABLE IF NOT EXISTS brain_scheduler_events(run_id TEXT NOT NULL REFERENCES brain_scheduler_runs(run_id),seq INTEGER NOT NULL,execution_id TEXT,source_sequence INTEGER,body TEXT NOT NULL,PRIMARY KEY(run_id,seq),UNIQUE(run_id,execution_id,source_sequence))",
    ]{conn.execute(sql,()).await?;}
    Ok(())
}
