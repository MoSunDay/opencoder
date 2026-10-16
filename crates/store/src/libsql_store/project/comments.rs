//! Ordered, idempotent note persistence under the project's existing write lock.
use crate::project::comments::{Comment, CommentError, CommentPage, CommentQuery, NewComment};
use anyhow::{Context, Result};
use libsql::{params, Connection};

const COLS: &str = "seq,todo_id,request_id,body_md,author_kind,author_id,author_name,capability_kind,submitted_by,execution_id,created_at";

pub(crate) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS project_todo_comments (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        todo_id TEXT NOT NULL, request_id TEXT NOT NULL, body_md TEXT NOT NULL,
        author_kind TEXT NOT NULL CHECK(author_kind IN ('user','capability')),
        author_id TEXT NOT NULL, author_name TEXT NOT NULL, capability_kind TEXT,
        submitted_by TEXT NOT NULL, execution_id TEXT, created_at INTEGER NOT NULL,
        UNIQUE(todo_id,submitted_by,request_id))",
        (),
    )
    .await?;
    conn.execute("CREATE INDEX IF NOT EXISTS project_todo_comments_page ON project_todo_comments(todo_id,seq)", ()).await?;
    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS project_todo_comments_delete
        AFTER DELETE ON project_todos BEGIN
        DELETE FROM project_todo_comments WHERE todo_id=OLD.id; END",
        (),
    )
    .await?;
    Ok(())
}

fn record(row: &libsql::Row) -> Result<Comment> {
    Ok(Comment {
        seq: row.get(0)?,
        entry: NewComment {
            todo_id: row.get(1)?,
            request_id: row.get(2)?,
            body_md: row.get(3)?,
            author_kind: row.get(4)?,
            author_id: row.get(5)?,
            author_name: row.get(6)?,
            capability_kind: row.get(7)?,
            submitted_by: row.get(8)?,
            execution_id: row.get(9)?,
            created_at: row.get(10)?,
        },
    })
}

pub async fn request(
    conn: &Connection,
    todo: &str,
    user: &str,
    id: &str,
) -> Result<Option<Comment>> {
    let mut rows = conn.query(&format!("SELECT {COLS} FROM project_todo_comments WHERE todo_id=? AND submitted_by=? AND request_id=?"), params![todo,user,id]).await?;
    rows.next().await?.as_ref().map(record).transpose()
}

pub async fn page(
    conn: &Connection,
    todo: &str,
    query: CommentQuery,
) -> Result<Option<CommentPage>> {
    anyhow::ensure!(query.valid(), "invalid comment pagination");
    if conn
        .query("SELECT 1 FROM project_todos WHERE id=?", params![todo])
        .await?
        .next()
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let limit = query.limit.unwrap_or(50);
    let (comparison, cursor, order) = match (query.before_seq, query.after_seq) {
        (Some(seq), _) => ("AND seq<?", seq, "DESC"),
        (_, Some(seq)) => ("AND seq>?", seq, "ASC"),
        _ => ("AND seq>?", 0, "DESC"),
    };
    let mut rows = conn.query(&format!("SELECT {COLS} FROM project_todo_comments WHERE todo_id=? {comparison} ORDER BY seq {order} LIMIT ?"), params![todo,cursor,(limit+1) as i64]).await?;
    let mut comments = vec![];
    while let Some(row) = rows.next().await? {
        comments.push(record(&row)?);
    }
    let has_more = comments.len() > limit;
    comments.truncate(limit);
    if order == "DESC" {
        comments.reverse();
    }
    Ok(Some(CommentPage { comments, has_more }))
}

pub async fn append(conn: &Connection, entry: &NewComment) -> Result<Comment> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async {
        if conn.query("SELECT 1 FROM project_todos WHERE id=?", params![entry.todo_id.as_str()]).await?.next().await?.is_none() {
            return Err(CommentError::MissingTodo.into());
        }
        if let Some(old) = request(conn, &entry.todo_id, &entry.submitted_by, &entry.request_id).await? {
            let capability = (entry.author_kind == "capability").then_some(entry.author_id.as_str());
            if !old.matches_input(&entry.body_md, capability, entry.execution_id.as_deref()) { return Err(CommentError::Conflict.into()); }
            return Ok(old);
        }
        if let Some(execution) = &entry.execution_id {
            let mut links = conn.query("SELECT capability_id FROM project_todo_executions WHERE todo_id=? AND execution_id=?", params![entry.todo_id.as_str(),execution.as_str()]).await?;
            let row = links.next().await?.ok_or(CommentError::InvalidSource)?;
            let capability: Option<String> = row.get(0)?;
            if entry.author_kind == "capability" && capability.as_ref().is_some_and(|id| id != &entry.author_id) { return Err(CommentError::InvalidSource.into()); }
        }
        conn.execute("INSERT INTO project_todo_comments(todo_id,request_id,body_md,author_kind,author_id,author_name,capability_kind,submitted_by,execution_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)", params![entry.todo_id.as_str(),entry.request_id.as_str(),entry.body_md.as_str(),entry.author_kind.as_str(),entry.author_id.as_str(),entry.author_name.as_str(),entry.capability_kind.as_deref(),entry.submitted_by.as_str(),entry.execution_id.as_deref(),entry.created_at]).await?;
        conn.execute("UPDATE project_todos SET updated_at=MAX(updated_at,?) WHERE id=?", params![entry.created_at,entry.todo_id.as_str()]).await?;
        request(conn, &entry.todo_id, &entry.submitted_by, &entry.request_id).await?.context("inserted comment is missing")
    }).await
}
