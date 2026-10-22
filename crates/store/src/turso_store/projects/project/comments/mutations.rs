//! Version-checked mutations preserve signatures and append-request identities.
use super::{record, Comment, CommentError, COLS, SOURCE};
use crate::project::comments::CommentMutation;
use anyhow::Result;
use turso::{params, Connection};

pub(super) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS project_todo_comment_state (
        seq INTEGER PRIMARY KEY, todo_id TEXT NOT NULL, submitted_by TEXT NOT NULL,
        request_id TEXT NOT NULL, original_body_md TEXT NOT NULL,
        revision INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted INTEGER NOT NULL,
        UNIQUE(todo_id,submitted_by,request_id))",
        (),
    )
    .await?;
    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS project_todo_comment_state_delete
        AFTER DELETE ON project_todos BEGIN
        DELETE FROM project_todo_comment_state WHERE todo_id=OLD.id; END",
        (),
    )
    .await?;
    Ok(())
}

pub(super) async fn check_deleted_request(
    conn: &Connection,
    todo: &str,
    user: &str,
    request: &str,
) -> Result<()> {
    if conn.query(
        "SELECT 1 FROM project_todo_comment_state WHERE todo_id=? AND submitted_by=? AND request_id=? AND deleted=1",
        params![todo,user,request],
    ).await?.next().await?.is_some() {
        return Err(CommentError::Deleted.into());
    }
    Ok(())
}

async fn current(conn: &Connection, todo: &str, seq: i64) -> Result<Option<Comment>> {
    let mut rows = conn
        .query(
            &format!("SELECT {COLS} FROM {SOURCE} WHERE c.todo_id=? AND c.seq=?"),
            params![todo, seq],
        )
        .await?;
    rows.next().await?.as_ref().map(record).transpose()
}

fn authorize(change: &CommentMutation, submitted_by: &str) -> Result<()> {
    if !change.actor.can_edit() || (!change.actor.is_admin() && change.actor.name != submitted_by) {
        return Err(CommentError::Forbidden.into());
    }
    Ok(())
}

async fn deleted_retry(conn: &Connection, change: &CommentMutation) -> Result<Option<Comment>> {
    let mut rows = conn.query(
        "SELECT submitted_by,revision FROM project_todo_comment_state WHERE todo_id=? AND seq=? AND deleted=1",
        params![change.todo_id.as_str(),change.seq],
    ).await?;
    let row = rows.next().await?.ok_or(CommentError::MissingComment)?;
    authorize(change, &row.get::<String>(0)?)?;
    if change.body_md.is_some() {
        return Err(CommentError::MissingComment.into());
    }
    if row.get::<i64>(1)? != change.expected_revision + 1 {
        return Err(CommentError::RevisionConflict.into());
    }
    Ok(None)
}

pub(crate) async fn apply(conn: &Connection, change: &CommentMutation) -> Result<Option<Comment>> {
    if change.seq <= 0
        || change.expected_revision <= 0
        || change.expected_revision == i64::MAX
        || change
            .body_md
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 65536)
    {
        return Err(CommentError::InvalidInput.into());
    }
    super::super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async {
        if conn.query("SELECT 1 FROM project_todos WHERE id=?", params![change.todo_id.as_str()]).await?.next().await?.is_none() {
            return Err(CommentError::MissingTodo.into());
        }
        let Some(old) = current(conn, &change.todo_id, change.seq).await? else {
            return deleted_retry(conn, change).await;
        };
        authorize(change, &old.entry.submitted_by)?;
        if old.revision != change.expected_revision {
            return Err(CommentError::RevisionConflict.into());
        }
        if change.body_md.as_deref() == Some(old.entry.body_md.as_str()) {
            return Ok(Some(old));
        }
        let original = old.original_body_md.as_deref().unwrap_or(&old.entry.body_md);
        conn.execute(
            "INSERT INTO project_todo_comment_state(seq,todo_id,submitted_by,request_id,original_body_md,revision,updated_at,deleted)
            VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(seq) DO UPDATE SET revision=excluded.revision,updated_at=excluded.updated_at,deleted=excluded.deleted",
            params![old.seq,change.todo_id.as_str(),old.entry.submitted_by.as_str(),old.entry.request_id.as_str(),original,old.revision+1,change.now,i64::from(change.body_md.is_none())],
        ).await?;
        if let Some(body) = &change.body_md {
            conn.execute("UPDATE project_todo_comments SET body_md=? WHERE todo_id=? AND seq=?",
                params![body.as_str(),change.todo_id.as_str(),old.seq]).await?;
        } else {
            conn.execute("DELETE FROM project_todo_comments WHERE todo_id=? AND seq=?",
                params![change.todo_id.as_str(),old.seq]).await?;
        }
        conn.execute("UPDATE project_todos SET updated_at=MAX(updated_at,?) WHERE id=?",
            params![change.now,change.todo_id.as_str()]).await?;
        current(conn, &change.todo_id, old.seq).await
    }).await
}
