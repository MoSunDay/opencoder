//! Check acceptance under the same write transaction as the TODO patch.
use crate::project::acceptance::{AcceptanceConflict, TodoAcceptance};
use anyhow::Result;
use turso::{params, Connection};

pub(super) async fn check(conn: &Connection, id: &str, expected: &TodoAcceptance) -> Result<()> {
    let todo = super::super::project_runs::get_todo(conn, id).await?;
    anyhow::ensure!(
        todo.as_ref()
            .is_some_and(|todo| expected.matches_todo(todo)),
        AcceptanceConflict
    );
    let mut rows = conn
        .query(
            "SELECT 1 FROM project_todos t JOIN project_initiatives i ON i.id=t.initiative_id \
         JOIN project_goals g ON g.id=i.goal_id WHERE t.id=? AND g.id=?",
            params![id, expected.project_id.as_str()],
        )
        .await?;
    anyhow::ensure!(rows.next().await?.is_some(), AcceptanceConflict);
    let latest = super::super::project_links::list(conn, id)
        .await?
        .into_iter()
        .max_by(|a, b| (a.created_at, &a.execution_id).cmp(&(b.created_at, &b.execution_id)));
    anyhow::ensure!(
        latest.is_some_and(|link| link.execution_id == expected.execution_id
            && link.capability_id.as_deref() == Some(expected.capability_id.as_str())),
        AcceptanceConflict
    );
    for dependency in &expected.required_done {
        anyhow::ensure!(dependency != id, AcceptanceConflict);
        let mut rows = conn
            .query(
                "SELECT 1 FROM project_todos t JOIN project_initiatives i ON i.id=t.initiative_id \
             WHERE t.id=? AND i.goal_id=? AND t.board_status='done'",
                params![dependency.as_str(), expected.project_id.as_str()],
            )
            .await?;
        anyhow::ensure!(rows.next().await?.is_some(), AcceptanceConflict);
    }
    if expected.all_other_todos {
        let mut rows = conn
            .query(
                "SELECT 1 FROM project_todos t JOIN project_initiatives i ON i.id=t.initiative_id \
             WHERE i.goal_id=? AND t.id<>? AND t.board_status<>'done' LIMIT 1",
                params![expected.project_id.as_str(), id],
            )
            .await?;
        anyhow::ensure!(rows.next().await?.is_none(), AcceptanceConflict);
    }
    Ok(())
}
