use super::super::Arg;
use super::io::{exec, query};
use anyhow::Result;
use sqlx::{MySqlConnection, MySqlPool};

pub(in crate::sql_store) async fn reorder_todos(
    pool: &MySqlPool,
    sr: bool,
    initiative_id: Option<&str>,
    status: &str,
    ids: &[String],
    now: i64,
) -> Result<()> {
    anyhow::ensure!(
        !ids.is_empty() && ids.len() <= 1000,
        "invalid TODO reorder size"
    );
    let mut seen = std::collections::HashSet::new();
    anyhow::ensure!(ids.iter().all(|id| seen.insert(id)), "duplicate TODO id");
    let mut conn = pool.acquire().await?;
    if !sr {
        sqlx::query("START TRANSACTION").execute(&mut *conn).await?;
    }
    let result = apply(&mut conn, sr, initiative_id, status, ids, now).await;
    if !sr {
        sqlx::query(if result.is_ok() { "COMMIT" } else { "ROLLBACK" })
            .execute(&mut *conn)
            .await?;
    }
    result
}

async fn apply(
    conn: &mut MySqlConnection,
    sr: bool,
    initiative_id: Option<&str>,
    status: &str,
    ids: &[String],
    now: i64,
) -> Result<()> {
    let marks = vec!["?"; ids.len()].join(",");
    let scope = format!("id IN ({marks}) AND initiative_id <=> ?");
    let mut selected: Vec<_> = ids.iter().cloned().map(Arg::Text).collect();
    selected.push(Arg::TextOrNull(initiative_id.map(str::to_owned)));
    let suffix = if sr { "" } else { " FOR UPDATE" };
    let found = query(
        conn,
        sr,
        &format!("SELECT id FROM project_todos WHERE {scope}{suffix}"),
        &selected,
    )
    .await?;
    anyhow::ensure!(
        found.len() == ids.len(),
        "TODO reorder contains missing or foreign id"
    );
    let mut args = vec![Arg::Text(status.into())];
    let mut cases = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        cases.push("WHEN id = ? THEN ?");
        args.extend([Arg::Text(id.clone()), Arg::Int((index + 1) as i64 * 1000)]);
    }
    args.push(Arg::Int(now));
    args.extend(selected);
    exec(conn, sr, &format!("UPDATE project_todos SET board_status=?,position=CASE {} END,updated_at=? WHERE {scope}", cases.join(" ")), args).await?;
    Ok(())
}
