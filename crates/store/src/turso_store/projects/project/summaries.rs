//! Bounded summary history. Writes share the catalog transaction.
use crate::project::summary::{SummaryScope, SummaryVersion, SUMMARY_HISTORY_LIMIT};
use anyhow::Result;
use turso::{params, Connection};

fn table(scope: SummaryScope) -> &'static str {
    match scope {
        SummaryScope::Project => "project_goals",
        SummaryScope::Initiative => "project_initiatives",
    }
}

pub(crate) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS project_summary_versions (
            scope_type TEXT NOT NULL, scope_id TEXT NOT NULL,
            version INTEGER NOT NULL, current_summary TEXT NOT NULL,
            created_at INTEGER NOT NULL, restored_from INTEGER,
            PRIMARY KEY(scope_type, scope_id, version))",
        (),
    )
    .await?;
    for scope in [SummaryScope::Project, SummaryScope::Initiative] {
        conn.execute(
            &format!(
                "INSERT INTO project_summary_versions
             SELECT ?1, id, 1, current_summary, COALESCE(summary_updated_at, created_at), NULL
             FROM {} AS owner WHERE NOT EXISTS (
                SELECT 1 FROM project_summary_versions WHERE scope_type=?1 AND scope_id=owner.id)",
                table(scope)
            ),
            params![scope.as_str()],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn append(
    conn: &Connection,
    scope: SummaryScope,
    id: &str,
    text: &str,
    now: i64,
    restored_from: Option<i64>,
) -> Result<SummaryVersion> {
    let mut rows = conn.query(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM project_summary_versions WHERE scope_type=? AND scope_id=?",
        params![scope.as_str(), id],
    ).await?;
    let version: i64 = rows.next().await?.unwrap().get(0)?;
    drop(rows);
    conn.execute(
        "INSERT INTO project_summary_versions VALUES(?,?,?,?,?,?)",
        params![scope.as_str(), id, version, text.trim(), now, restored_from],
    )
    .await?;
    conn.execute(
        "DELETE FROM project_summary_versions WHERE scope_type=? AND scope_id=? AND version<=?",
        params![scope.as_str(), id, version - SUMMARY_HISTORY_LIMIT],
    )
    .await?;
    Ok(SummaryVersion {
        version,
        current_summary: text.trim().into(),
        created_at: now,
        restored_from,
    })
}

pub(super) async fn remove(conn: &Connection, scope: SummaryScope, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM project_summary_versions WHERE scope_type=? AND scope_id=?",
        params![scope.as_str(), id],
    )
    .await?;
    Ok(())
}

pub(super) async fn list(
    conn: &Connection,
    scope: SummaryScope,
    id: &str,
) -> Result<Option<Vec<SummaryVersion>>> {
    if !super::exists(
        conn,
        &format!("SELECT 1 FROM {} WHERE id=?", table(scope)),
        id,
    )
    .await?
    {
        return Ok(None);
    }
    let mut rows = conn
        .query(
            "SELECT version,current_summary,created_at,restored_from FROM project_summary_versions
         WHERE scope_type=? AND scope_id=? ORDER BY version DESC LIMIT ?",
            params![scope.as_str(), id, SUMMARY_HISTORY_LIMIT],
        )
        .await?;
    let mut result = vec![];
    while let Some(row) = rows.next().await? {
        result.push(SummaryVersion {
            version: row.get(0)?,
            current_summary: row.get(1)?,
            created_at: row.get(2)?,
            restored_from: row.get(3)?,
        });
    }
    Ok(Some(result))
}

pub(super) async fn restore(
    conn: &Connection,
    scope: SummaryScope,
    id: &str,
    version: i64,
    now: i64,
) -> Result<Option<SummaryVersion>> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let mut rows = conn.query(
            "SELECT current_summary FROM project_summary_versions WHERE scope_type=? AND scope_id=? AND version=?",
            params![scope.as_str(), id, version],
        ).await?;
        let Some(row) = rows.next().await? else { return Ok(None) };
        let text: String = row.get(0)?;
        drop(rows);
        let changed = conn.execute(&format!(
            "UPDATE {} SET current_summary=?,summary_updated_at=?,updated_at=? WHERE id=?", table(scope)),
            params![text.as_str(), now, now, id]).await?;
        if changed == 0 { return Ok(None) }
        Ok(Some(append(conn, scope, id, &text, now, Some(version)).await?))
    }).await
}
