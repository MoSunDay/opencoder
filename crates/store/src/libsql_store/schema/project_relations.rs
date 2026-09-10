//! v23: independent milestones and one-time classification of legacy backlog.
//! Runs inside the bootstrap transaction; no business record is discarded.
use anyhow::{Result, ensure};
use libsql::{Connection, params};

pub(super) async fn migrate(conn: &Connection) -> Result<()> {
    let mut columns = conn
        .query("PRAGMA table_info(project_milestones)", ())
        .await?;
    let mut required = false;
    while let Some(row) = columns.next().await? {
        if row.get::<String>(1)? == "goal_id" {
            required = row.get::<i64>(3)? != 0;
        }
    }
    drop(columns);
    if required {
        conn.execute(
            &super::CREATE_PROJECT_MILESTONES
                .replace("project_milestones (", "project_milestones_v23 ("),
            (),
        )
        .await?;
        conn.execute(
            "INSERT INTO project_milestones_v23 SELECT * FROM project_milestones",
            (),
        )
        .await?;
        let mut diff = conn
            .query(
                "SELECT * FROM project_milestones EXCEPT SELECT * FROM project_milestones_v23",
                (),
            )
            .await?;
        ensure!(
            diff.next().await?.is_none(),
            "milestone migration did not preserve every field"
        );
        drop(diff);
        conn.execute("DROP TABLE project_milestones", ()).await?;
        conn.execute(
            "ALTER TABLE project_milestones_v23 RENAME TO project_milestones",
            (),
        )
        .await?;
    }
    let mut backlog = conn
        .query(
            "SELECT 1 FROM project_todos WHERE milestone_id IS NULL LIMIT 1",
            (),
        )
        .await?;
    let has_backlog = backlog.next().await?.is_some();
    drop(backlog);
    if has_backlog {
        let id = format!("pm-{}", ulid::Ulid::new());
        let now = opencoder_core::message::now_ms();
        conn.execute(
            "INSERT INTO project_milestones (id,goal_id,title,detail_md,status,sort_key,created_at,updated_at)
             VALUES (?,NULL,'待归类',NULL,'planned',0,?,?)", params![id.as_str(), now, now],
        ).await?;
        conn.execute(
            "UPDATE project_todos SET milestone_id = ? WHERE milestone_id IS NULL",
            params![id],
        )
        .await?;
    }
    Ok(())
}
