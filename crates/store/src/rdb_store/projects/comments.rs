//! Ordered, idempotent comment persistence (`pcm/<todo>/<seq:020>` with a
//! `UNIQUE(todo_id, submitted_by, request_id)` index). A deleted comment
//! keeps its row (flagged) so old append requests can never resurrect it.

use anyhow::Result;

use crate::project::comments::{
    Comment, CommentError, CommentMutation, CommentPage, CommentQuery, NewComment,
};
use crate::rdb_store::kv;
use crate::rdb_store::projects::keys::{comment_key, comment_prefix, comment_request_index};
use crate::rdb_store::projects::{links, todos};
use crate::rdb_store::seq;
use crate::rdb_store::{index, tx};

const BODY_LIMIT: usize = 65536;

/// Decode one row keyed by `(key, value)`; the seq comes from the key.
fn row(
    key: &[u8],
    value: &[u8],
    prefix: &[u8],
) -> Result<(i64, crate::rdb_store::projects::keys::CommentRow)> {
    let seq =
        seq::parse_ord(key, prefix).ok_or_else(|| anyhow::anyhow!("comment key carries no seq"))?;
    Ok((seq, kv::decode(value)?))
}

/// All rows of one todo (including deleted ones), ascending by seq.
fn rows(
    store: &rdb::store::Store,
    todo: &str,
) -> Result<Vec<(i64, crate::rdb_store::projects::keys::CommentRow)>> {
    let prefix = comment_prefix(todo);
    let mut out = Vec::new();
    for (key, value) in tx::scan_prefix(store, &prefix)? {
        out.push(row(&key, &value, &prefix)?);
    }
    Ok(out)
}

fn authorize(change: &CommentMutation, submitted_by: &str) -> Result<()> {
    if !change.actor.can_edit() || (!change.actor.is_admin() && change.actor.name != submitted_by) {
        return Err(CommentError::Forbidden.into());
    }
    Ok(())
}

/// The comment behind one append identity: `Err(Deleted)` when that
/// identity was used for a comment that has since been deleted.
fn find_request(
    store: &rdb::store::Store,
    todo: &str,
    user: &str,
    request: &str,
) -> Result<Option<(i64, crate::rdb_store::projects::keys::CommentRow)>> {
    Ok(rows(store, todo)?
        .into_iter()
        .find(|(_, row)| row.entry.submitted_by == user && row.entry.request_id == request))
}

pub fn request(
    store: &rdb::store::Store,
    todo: &str,
    user: &str,
    request: &str,
) -> Result<Option<Comment>> {
    match find_request(store, todo, user, request)? {
        Some((_, row)) if row.deleted => Err(CommentError::Deleted.into()),
        Some((seq, row)) => Ok(Some(row.to_comment(seq))),
        None => Ok(None),
    }
}

pub fn page(
    store: &rdb::store::Store,
    todo: &str,
    query: CommentQuery,
) -> Result<Option<CommentPage>> {
    anyhow::ensure!(query.valid(), "invalid comment pagination");
    if todos::get(store, todo)?.is_none() {
        return Ok(None);
    }
    let limit = query.limit.unwrap_or(50);
    let mut live: Vec<Comment> = rows(store, todo)?
        .into_iter()
        .filter(|(_, row)| !row.deleted)
        .map(|(seq, row)| row.to_comment(seq))
        .collect();
    let (cursor, ascending) = match (query.before_seq, query.after_seq) {
        (Some(seq), _) => (Some(seq), false),
        (_, Some(seq)) => (Some(seq), true),
        // First page: the newest window, i.e. `seq > 0` ordered DESC.
        _ => (None, false),
    };
    live.retain(|comment| match cursor {
        Some(seq) if ascending => comment.seq > seq,
        Some(seq) => comment.seq < seq,
        None => true,
    });
    live.sort_by_key(|comment| comment.seq);
    if !ascending {
        live.reverse();
    }
    let has_more = live.len() > limit;
    live.truncate(limit);
    if !ascending {
        live.reverse();
    }
    Ok(Some(CommentPage {
        comments: live,
        has_more,
    }))
}

/// Idempotent append keyed by `(todo, submitted_by, request_id)`; a
/// different input under the same identity is `Conflict`, and a deleted
/// comment permanently burns its append identity.
pub fn append(store: &rdb::store::Store, entry: &NewComment) -> Result<Comment> {
    if todos::get(store, &entry.todo_id)?.is_none() {
        return Err(CommentError::MissingTodo.into());
    }
    if let Some(found) = find_request(
        store,
        &entry.todo_id,
        &entry.submitted_by,
        &entry.request_id,
    )? {
        if found.1.deleted {
            return Err(CommentError::Deleted.into());
        }
        let old = found.1.to_comment(found.0);
        let capability = (entry.author_kind == "capability").then_some(entry.author_id.as_str());
        if !old.matches_input(&entry.body_md, capability, entry.execution_id.as_deref()) {
            return Err(CommentError::Conflict.into());
        }
        return Ok(old);
    }
    if let Some(execution) = &entry.execution_id {
        let link = links::list(store, &entry.todo_id)?
            .into_iter()
            .find(|link| &link.execution_id == execution)
            .ok_or(CommentError::InvalidSource)?;
        if entry.author_kind == "capability"
            && link
                .capability_id
                .as_ref()
                .is_some_and(|id| id != &entry.author_id)
        {
            return Err(CommentError::InvalidSource.into());
        }
    }
    let mut batch = rocksdb::WriteBatch::default();
    let seq = seq::next_global(store, &mut batch, "pcm")?;
    let key = comment_key(&entry.todo_id, seq);
    index::claim_unique(
        store,
        &mut batch,
        &comment_request_index(&entry.todo_id, &entry.submitted_by, &entry.request_id),
        &key,
        "project_todo_comments.todo_id, project_todo_comments.submitted_by, project_todo_comments.request_id",
    )?;
    let stored = crate::rdb_store::projects::keys::CommentRow::new(entry);
    tx::put(&mut batch, &key, &stored)?;
    if let Some(mut todo) = todos::get(store, &entry.todo_id)? {
        if todo.updated_at < entry.created_at {
            todo.updated_at = entry.created_at;
            tx::put(
                &mut batch,
                &crate::rdb_store::projects::keys::todo_key(&todo.id),
                &todo,
            )?;
        }
    }
    tx::commit(store, batch)?;
    Ok(stored.to_comment(seq))
}

/// Version-checked edit/delete; `None` after a successful delete.
pub fn mutate(store: &rdb::store::Store, change: &CommentMutation) -> Result<Option<Comment>> {
    if change.seq <= 0
        || change.expected_revision <= 0
        || change.expected_revision == i64::MAX
        || change
            .body_md
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > BODY_LIMIT)
    {
        return Err(CommentError::InvalidInput.into());
    }
    if todos::get(store, &change.todo_id)?.is_none() {
        return Err(CommentError::MissingTodo.into());
    }
    let Some((seq, mut stored)) = rows(store, &change.todo_id)?
        .into_iter()
        .find(|(seq, _)| *seq == change.seq)
    else {
        return Err(CommentError::MissingComment.into());
    };
    authorize(change, &stored.entry.submitted_by)?;
    if stored.deleted {
        // A deleted comment only answers matching no-op retries.
        if change.body_md.is_some() {
            return Err(CommentError::MissingComment.into());
        }
        if stored.revision != change.expected_revision + 1 {
            return Err(CommentError::RevisionConflict.into());
        }
        return Ok(None);
    }
    if stored.revision != change.expected_revision {
        return Err(CommentError::RevisionConflict.into());
    }
    let body = change.body_md.clone();
    if body.as_deref() == Some(stored.entry.body_md.as_str()) {
        return Ok(Some(stored.to_comment(seq)));
    }
    if stored.original_body_md.is_none() {
        stored.original_body_md = Some(stored.entry.body_md.clone());
    }
    if let Some(body) = &body {
        stored.entry.body_md = body.clone();
    } else {
        stored.deleted = true;
    }
    stored.revision += 1;
    stored.updated_at = change.now;
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &comment_key(&change.todo_id, seq), &stored)?;
    if let Some(mut todo) = todos::get(store, &change.todo_id)? {
        if todo.updated_at < change.now {
            todo.updated_at = change.now;
            tx::put(
                &mut batch,
                &crate::rdb_store::projects::keys::todo_key(&todo.id),
                &todo,
            )?;
        }
    }
    tx::commit(store, batch)?;
    Ok((!stored.deleted).then(|| stored.to_comment(seq)))
}
