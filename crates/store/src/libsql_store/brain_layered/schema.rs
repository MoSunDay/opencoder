use anyhow::Result;
use libsql::Connection;
pub(crate) async fn initialize(conn: &Connection) -> Result<()> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS brain_layered_runs(run_id TEXT PRIMARY KEY,generation INTEGER NOT NULL,body TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS brain_layered_operations(execution_id TEXT PRIMARY KEY,operation_id TEXT NOT NULL UNIQUE,run_id TEXT NOT NULL REFERENCES brain_layered_runs(run_id),layer INTEGER NOT NULL,body TEXT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS brain_layered_operations_layer ON brain_layered_operations(run_id,layer)",
        "CREATE TABLE IF NOT EXISTS brain_layered_events(run_id TEXT NOT NULL REFERENCES brain_layered_runs(run_id),seq INTEGER NOT NULL,execution_id TEXT,source_sequence INTEGER,body TEXT NOT NULL,PRIMARY KEY(run_id,seq),UNIQUE(run_id,execution_id,source_sequence))",
    ] {
        conn.execute(sql, ()).await?;
    }
    Ok(())
}
