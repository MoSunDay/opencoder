//! Replace project execution caches with a reference-only table in the
//! bootstrap transaction. Execution bodies remain on their owning nodes.
use anyhow::{ensure, Result};
use libsql::Connection;

pub(super) async fn migrate(conn: &Connection) -> Result<()> {
    for (name, definition) in [
        ("kind", "TEXT NOT NULL DEFAULT ''"),
        ("name", "TEXT NOT NULL DEFAULT ''"),
        ("capability_id", "TEXT"),
    ] {
        super::add_column_if_absent(conn, "project_todo_executions", name, definition).await?;
    }
    conn.execute(
        &super::CREATE_PROJECT_TODO_EXECUTIONS.replace(
            "project_todo_executions (",
            "project_todo_executions_refs (",
        ),
        (),
    )
    .await?;
    let columns = "todo_id,execution_id,created_at,kind,name,capability_id";
    conn.execute(
        &format!("INSERT INTO project_todo_executions_refs ({columns}) SELECT {columns} FROM project_todo_executions"), (),
    ).await?;
    let mut missing = conn.query(
        &format!("SELECT {columns} FROM project_todo_executions EXCEPT SELECT {columns} FROM project_todo_executions_refs"), (),
    ).await?;
    ensure!(
        missing.next().await?.is_none(),
        "project execution index migration lost references"
    );
    drop(missing);
    conn.execute("DROP TABLE project_todo_executions", ())
        .await?;
    conn.execute(
        "ALTER TABLE project_todo_executions_refs RENAME TO project_todo_executions",
        (),
    )
    .await?;
    Ok(())
}
