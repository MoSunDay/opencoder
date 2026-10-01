//! v32: one initiative hierarchy, preserving TODOs and immutable execution data.
use super::{column_exists, table_exists};
use anyhow::{ensure, Result};
use libsql::Connection;

pub(super) async fn initialize_tags(conn: &Connection) -> Result<()> {
    conn.execute("CREATE TABLE IF NOT EXISTS project_tags (id TEXT PRIMARY KEY, scope_type TEXT NOT NULL, scope_id TEXT NOT NULL, name TEXT NOT NULL, UNIQUE(scope_type,scope_id,name))", ()).await?;
    conn.execute("CREATE TABLE IF NOT EXISTS project_todo_tags (todo_id TEXT NOT NULL, tag_id TEXT NOT NULL, PRIMARY KEY(todo_id,tag_id))", ()).await?;
    Ok(())
}

pub(super) async fn upgrade(conn: &Connection) -> Result<()> {
    if table_exists(conn, "project_milestones").await? {
        conn.execute("INSERT OR IGNORE INTO project_initiatives (id,goal_id,title,detail_md,status,sort_key,created_at,updated_at) SELECT id,goal_id,title,detail_md,status,sort_key,created_at,updated_at FROM project_milestones WHERE kind='initiative'", ()).await?;
        let mut diff = conn.query("SELECT id,goal_id,title,detail_md,status,sort_key,created_at,updated_at FROM project_milestones WHERE kind='initiative' EXCEPT SELECT id,goal_id,title,detail_md,status,sort_key,created_at,updated_at FROM project_initiatives", ()).await?;
        ensure!(
            diff.next().await?.is_none(),
            "initiative migration would lose data"
        );
        drop(diff);
        if column_exists(conn, "project_todos", "milestone_id").await? {
            conn.execute("UPDATE project_todos SET milestone_id=NULL WHERE milestone_id IN (SELECT id FROM project_milestones WHERE kind <> 'initiative')", ()).await?;
        }
        conn.execute("DROP TABLE project_milestones", ()).await?;
    }
    if column_exists(conn, "project_todos", "milestone_id").await? {
        conn.execute(
            "ALTER TABLE project_todos RENAME COLUMN milestone_id TO initiative_id",
            (),
        )
        .await?;
    }
    conn.execute("DROP INDEX IF EXISTS idx_project_todos_milestone", ())
        .await?;
    initialize_tags(conn).await
}
