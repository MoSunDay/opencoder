use anyhow::Result;
use turso::Connection;

pub(crate) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute("CREATE TABLE IF NOT EXISTS inventory_source_changes(seq INTEGER PRIMARY KEY AUTOINCREMENT,source TEXT NOT NULL,id TEXT NOT NULL)",()).await?;
    for (table, source, key, columns) in [
        (
            "sessions",
            "session",
            "id",
            "id,kind,title,created_at,task_type",
        ),
        (
            "subagent_tasks",
            "session",
            "child_session_id",
            "child_session_id",
        ),
        (
            "project_todo_runs",
            "project",
            "id",
            "id,todo_id,created_at,status",
        ),
    ] {
        for event in ["INSERT", "DELETE", "UPDATE"] {
            let update = if event == "UPDATE" {
                format!("UPDATE OF {columns}")
            } else {
                event.into()
            };
            let condition = if event == "UPDATE" {
                format!(
                    " WHEN {}",
                    columns
                        .split(',')
                        .map(|column| format!("OLD.{column} IS NOT NEW.{column}"))
                        .collect::<Vec<_>>()
                        .join(" OR ")
                )
            } else {
                String::new()
            };
            let mut statements = String::new();
            if event != "INSERT" {
                statements.push_str(&format!(
                    "INSERT INTO inventory_source_changes(source,id) VALUES('{source}',OLD.{key});"
                ));
            }
            if event != "DELETE" {
                statements.push_str(&format!(
                    "INSERT INTO inventory_source_changes(source,id) VALUES('{source}',NEW.{key});"
                ));
            }
            conn.execute(&format!("CREATE TRIGGER IF NOT EXISTS inventory_{table}_{} AFTER {update} ON {table}{condition} BEGIN {statements} DELETE FROM inventory_source_changes WHERE seq<(SELECT MAX(seq)-100000 FROM inventory_source_changes); END",event.to_lowercase()),()).await?;
        }
    }
    Ok(())
}
