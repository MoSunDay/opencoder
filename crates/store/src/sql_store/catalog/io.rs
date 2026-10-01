use super::super::{bind_args, inline_args, Arg};
use crate::project::{ProjectTag, ProjectTodoTag};
use anyhow::Result;
use sqlx::{Executor, MySqlConnection, MySqlPool, Row};
pub(super) fn query<'a>(
    conn: &'a mut MySqlConnection,
    starrocks: bool,
    sql: &'a str,
    args: &'a [Arg],
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<sqlx::mysql::MySqlRow>>> + Send + 'a>,
> {
    Box::pin(async move {
        if starrocks {
            let rendered = inline_args(sql, args)?;
            Ok((&mut *conn).fetch_all(rendered.as_str()).await?)
        } else {
            Ok(bind_args(sqlx::query(sql), args.to_vec())
                .fetch_all(&mut *conn)
                .await?)
        }
    })
}
pub(super) async fn exec(
    conn: &mut MySqlConnection,
    starrocks: bool,
    sql: &str,
    args: Vec<Arg>,
) -> Result<u64> {
    Ok(if starrocks {
        (&mut *conn)
            .execute(inline_args(sql, &args)?.as_str())
            .await?
    } else {
        bind_args(sqlx::query(sql), args).execute(conn).await?
    }
    .rows_affected())
}
pub(super) fn read_tag(row: &sqlx::mysql::MySqlRow) -> Result<ProjectTag> {
    Ok(ProjectTag {
        id: row.try_get("id")?,
        scope_type: row.try_get("scope_type")?,
        scope_id: row.try_get("scope_id")?,
        name: row.try_get("name")?,
    })
}
pub(super) fn read_link(row: &sqlx::mysql::MySqlRow) -> Result<ProjectTodoTag> {
    Ok(ProjectTodoTag {
        todo_id: row.try_get("todo_id")?,
        tag_id: row.try_get("tag_id")?,
    })
}
pub(in crate::sql_store) async fn tags(
    pool: &MySqlPool,
    starrocks: bool,
) -> Result<Vec<ProjectTag>> {
    super::super::exec_read_all(
        pool,
        starrocks,
        "SELECT id,scope_type,scope_id,name FROM project_tags ORDER BY name,id",
        &[],
    )
    .await?
    .iter()
    .map(read_tag)
    .collect()
}
pub(in crate::sql_store) async fn links(
    pool: &MySqlPool,
    starrocks: bool,
) -> Result<Vec<ProjectTodoTag>> {
    super::super::exec_read_all(
        pool,
        starrocks,
        "SELECT todo_id,tag_id FROM project_todo_tags ORDER BY todo_id,tag_id",
        &[],
    )
    .await?
    .iter()
    .map(read_link)
    .collect()
}

pub(super) async fn validate_tag_scope(
    conn: &mut MySqlConnection,
    sr: bool,
    tag: &ProjectTag,
    previous: &[ProjectTag],
) -> Result<()> {
    use crate::project::tags::TagError;
    let table = match tag.scope_type.as_str() {
        "project" => "project_goals",
        "initiative" => "project_initiatives",
        _ => anyhow::bail!(TagError::InvalidScope),
    };
    let suffix = if sr { "" } else { " FOR UPDATE" };
    anyhow::ensure!(
        !query(
            conn,
            sr,
            &format!("SELECT id FROM {table} WHERE id=?{suffix}"),
            &[Arg::Text(tag.scope_id.clone())]
        )
        .await?
        .is_empty(),
        TagError::InvalidScope
    );
    anyhow::ensure!(
        previous
            .iter()
            .filter(|t| t.id == tag.id)
            .all(|t| t.scope_type == tag.scope_type && t.scope_id == tag.scope_id),
        TagError::InvalidScope
    );
    Ok(())
}
