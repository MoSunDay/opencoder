//! Atomic board moves for project TODOs.

use anyhow::Result;
use libsql::{Connection, Value};

// One SQL statement moves and reorders a destination lane atomically.
pub async fn reorder_todos(
    conn: &Connection,
    initiative_id: Option<&str>,
    board_status: &str,
    ids: &[String],
    now_ms: i64,
) -> Result<()> {
    anyhow::ensure!(
        !ids.is_empty() && ids.len() <= 1000,
        "invalid TODO reorder size"
    );
    let mut seen = std::collections::HashSet::new();
    anyhow::ensure!(ids.iter().all(|id| seen.insert(id)), "duplicate TODO id");
    let mut values: Vec<Value> = vec![board_status.into()];
    let mut cases = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        cases.push("WHEN id = ? THEN ?".to_string());
        values.push(id.as_str().into());
        values.push(((index + 1) as i64 * 1000).into());
    }
    values.push(now_ms.into());
    values.extend(ids.iter().map(|id| Value::from(id.as_str())));
    values.push(initiative_id.map(Value::from).unwrap_or(Value::Null));
    let marks = vec!["?"; ids.len()].join(",");
    let sql = format!("UPDATE project_todos SET board_status = ?, position = CASE {} END, updated_at = ? WHERE id IN ({marks}) AND initiative_id IS ?", cases.join(" "));
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let changed = conn.execute(&sql, values).await?;
        anyhow::ensure!(
            changed == ids.len() as u64,
            "TODO reorder contains missing id"
        );
        Ok(())
    })
    .await
}
