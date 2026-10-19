//! Read-only candidates for catalog removal. Completed execution history is retained.
use super::FleetStore;
use anyhow::Result;
use serde_json::Value;

impl FleetStore {
    /// Conservative substring filtering happens in SQLite; callers inspect the JSON
    /// before treating a candidate as a reference. Missing indexes stay protected.
    pub async fn catalog_reference_candidates(&self, name: &str) -> Result<Vec<(String, Value)>> {
        anyhow::ensure!(!name.is_empty(), "catalog reference name is empty");
        let _guard = self.gate.lock().await;
        // Escaped/non-ASCII identifiers can have several JSON spellings. In
        // that case inspect every candidate instead of risking a false negative.
        let needle = if name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        {
            name
        } else {
            ""
        };
        let mut result = Vec::new();
        for query in [
            "SELECT 'definition:'||kind||':'||id,body FROM fleet_definitions WHERE instr(body,?1)>0",
            "SELECT 'plan:'||id||'@'||version,body FROM brain_plan_versions WHERE instr(body,?1)>0",
            "SELECT 'execution:'||a.id,a.assignment FROM execution_assignments a LEFT JOIN execution_index i ON i.id=a.id WHERE (i.id IS NULL OR i.status NOT IN ('done','error','cancelled')) AND instr(a.assignment,?1)>0",
            "SELECT 'dispatch:'||scope||':'||id,payload FROM dispatch_receipts WHERE phase NOT IN ('accepted','rejected','done','complete','completed','cancelled') AND instr(payload,?1)>0",
        ] {
            let mut rows = self.conn.query(query, [needle]).await?;
            while let Some(row) = rows.next().await? {
                result.push((row.get(0)?, serde_json::from_str(&row.get::<String>(1)?)?));
            }
        }
        Ok(result)
    }
}
