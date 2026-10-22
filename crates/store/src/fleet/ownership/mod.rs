//! Published owners use the immutable execution index. Only first reports need
//! the frozen assignment; native preparation cannot change an indexed owner.
use anyhow::{ensure, Result};
use opencoder_core::fleet::ExecutionIndex;
use turso::Connection;

#[cfg(test)]
mod tests;

pub(super) async fn validate_unpublished(conn: &Connection, encoded: &str) -> Result<()> {
    // The report owns BEGIN IMMEDIATE. Native prepare_assignment verifies both
    // the existing assignment and index before writing; deletion removes both
    // in one transaction. Therefore the scalar index remains authoritative
    // after first acceptance, including while older Servers are still writing.
    let mut rows = conn
        .query(
            "SELECT a.id,a.assignment,incoming.value FROM json_each(?1) incoming
             JOIN execution_assignments a ON a.id=CAST(json_extract(incoming.value,'$.id') AS TEXT)
             WHERE NOT EXISTS(SELECT 1 FROM execution_index i WHERE i.id=a.id)",
            [encoded],
        )
        .await?;
    #[derive(serde::Deserialize)]
    struct Owner {
        index: ExecutionIndex,
    }
    while let Some(row) = rows.next().await? {
        let id: String = row.get(0)?;
        let owner: Owner = serde_json::from_str(&row.get::<String>(1)?)?;
        let incoming: ExecutionIndex = serde_json::from_str(&row.get::<String>(2)?)?;
        ensure!(
            owner.index.id == id
                && owner.index.created_at == incoming.created_at
                && owner.index.kind == incoming.kind
                && owner.index.node_id == incoming.node_id,
            "execution ownership conflict: {id}"
        );
    }
    Ok(())
}
