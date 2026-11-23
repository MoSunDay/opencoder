//! Session listings (`/task`, execution inventory).
//!
//! Fast path: one statement walking the activity index. Turso fails a scan
//! as a whole as soon as it reaches a page-corrupt row (reported as
//! `Corrupt`, or as a worker panic on page shapes it cannot classify), so
//! on unreadable storage the listing degrades to two phases instead of
//! failing: the candidate ids come from the covering activity index (which
//! never reads table pages), then one point lookup per candidate skips the
//! rows that cannot be read. The damaged sessions stay hidden until the
//! database is repaired; every other session keeps listing.

use anyhow::{Context, Result};
use turso::{params_from_iter, Connection, Value};

use super::{extract_preview, normalize_agent, ACTIVITY_EXPR};
use crate::store::errors::is_unreadable_storage;
use crate::types::{
    SessionFilter, SessionListItem, TASK_TYPE_AGENT_STEP, TASK_TYPE_PARENT, TASK_TYPE_SUBAGENT,
};

/// Point-lookup fallback: candidates fetched per round. A page plus headroom
/// for rows the fallback filters out or cannot read.
const FALLBACK_BATCH_MIN: i64 = 100;
/// Covering activity key, valid on both index shapes (the legacy expression
/// index and the plain column index). `updated_at >= created_at` is a stored
/// invariant, so it equals `ACTIVITY_EXPR` and cursors stay interchangeable.
const COVERING_ACTIVITY: &str = "MAX(s.updated_at, s.created_at)";

const ITEM_COLUMNS: &str = "s.id, s.kind, s.title, s.agent, s.model, s.created_at, s.updated_at";
/// Short JSON needs no character-by-character substring copy. Preserve
/// substr's character limit and embedded-NUL behavior for all other rows.
const PREVIEW_SUBQUERY: &str = "(SELECT CASE WHEN octet_length(m.blocks_json) <= 8192 AND \
     instr(m.blocks_json, char(0)) = 0 THEN m.blocks_json ELSE substr(m.blocks_json, 1, 8192) END \
     FROM messages m WHERE m.session_id = s.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1)";

/// A listing as a reusable predicate plan: the fast path evaluates every
/// clause in one statement, the fallback evaluates the cursor on the covering
/// index and the row clauses per candidate.
struct ListPlan {
    limit: i64,
    /// Clauses that need the table row (workdir, search, task-type fence,
    /// lane fence); AND-combinable.
    row_clauses: Vec<String>,
    row_args: Vec<Value>,
    /// Keyset position `(activity_ts, id)` the page starts after.
    cursor: Option<(i64, String)>,
}

impl ListPlan {
    fn build(filter: &SessionFilter, include_execution_details: bool) -> Result<Self> {
        let mut row_clauses: Vec<String> = Vec::new();
        let mut row_args: Vec<Value> = Vec::new();

        if let Some(h) = &filter.workdir_hash {
            row_clauses.push("s.workdir_hash = ?".into());
            row_args.push(h.clone().into());
        }
        if let Some(s) = &filter.search {
            // Escape LIKE metacharacters (`\`, `%`, `_`) in the user-supplied
            // term so they match literally. `ESCAPE '\'` declares the escape.
            row_clauses.push(
                "(s.id LIKE ? ESCAPE '\\' OR COALESCE(s.title,'') LIKE ? ESCAPE '\\')".into(),
            );
            let escaped = s
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like = format!("%{escaped}%");
            row_args.push(like.clone().into());
            row_args.push(like.into());
        }
        // `include_subagents` widens the visible types from "top-level parents
        // only" to parents + subagent children. DAG Agent-step sessions are
        // durable execution detail and remain queryable by their session ID;
        // they are filtered from the chat lane by the control API's fence.
        if filter.include_subagents && !include_execution_details {
            row_clauses.push(format!(
                "s.task_type IN ('{TASK_TYPE_PARENT}','{TASK_TYPE_SUBAGENT}')"
            ));
        } else {
            row_clauses.push(format!(
                "((s.task_type = '{TASK_TYPE_PARENT}' AND NOT EXISTS (SELECT 1 FROM subagent_tasks st WHERE st.child_session_id = s.id)) OR s.task_type = '{TASK_TYPE_AGENT_STEP}')"
            ));
        }
        // Lane fence: operator-tagged sessions are execution detail, never
        // chat history, so every lane hides them unless explicitly requested.
        // Legacy rows (kind NULL) keep the caller-side prefix resolution.
        match filter.kind.as_deref() {
            Some(kind) => {
                row_clauses.push("s.kind = ?".into());
                row_args.push(kind.into());
            }
            None => row_clauses.push("(s.kind IS NULL OR s.kind <> 'operator')".into()),
        }
        // A malformed cursor is a real error, not a silent fallback to page 1.
        let cursor = match &filter.cursor {
            Some(cursor) => Some(
                super::decode_cursor(cursor)
                    .ok_or_else(|| anyhow::anyhow!("invalid list cursor: {cursor}"))?,
            ),
            None => None,
        };
        Ok(Self {
            limit: i64::from(filter.limit.clamp(1, 500)),
            row_clauses,
            row_args,
            cursor,
        })
    }

    /// Append the keyset predicate on `activity` and bind its parameters.
    fn push_cursor(&self, sql: &mut String, args: &mut Vec<Value>, activity: &str) {
        if let Some((ts, id)) = &self.cursor {
            sql.push_str(&format!(
                "({activity} < ? OR ({activity} = ? AND s.id < ?))"
            ));
            args.push((*ts).into());
            args.push((*ts).into());
            args.push(id.clone().into());
        }
    }

    /// Append the row predicate (always true when there are no clauses) and
    /// bind its parameters.
    fn push_row_predicate(&self, sql: &mut String, args: &mut Vec<Value>) {
        if self.row_clauses.is_empty() {
            sql.push_str("1");
        } else {
            sql.push_str(&self.row_clauses.join(" AND "));
        }
        args.extend(self.row_args.iter().cloned());
    }
}

pub(super) async fn list(
    conn: &Connection,
    filter: &SessionFilter,
    include_execution_details: bool,
) -> Result<Vec<SessionListItem>> {
    let plan = ListPlan::build(filter, include_execution_details)?;
    match fetch_page(conn, &plan).await {
        // Unreadable storage degrades to point lookups; everything else (a
        // malformed cursor, a missing index) surfaces unchanged.
        Err(error) if is_unreadable_storage(&error) => {
            tracing::warn!(
                error = %error,
                "sessions table has unreadable rows; listing sessions through point lookups"
            );
            list_via_point_lookups(conn, &plan).await
        }
        other => other,
    }
}

/// Single-statement page: walk the activity index until the page is full
/// (the task-type index otherwise makes Turso sort every matching session
/// before LIMIT), then load previews for the page only.
async fn fetch_page(conn: &Connection, plan: &ListPlan) -> Result<Vec<SessionListItem>> {
    let mut sql = String::from(
        "WITH page AS MATERIALIZED (SELECT s.id, s.kind, s.title, s.agent, s.model, \
         s.created_at, s.updated_at, s.skill FROM sessions s INDEXED BY idx_sessions_activity WHERE ",
    );
    let mut args = Vec::new();
    plan.push_row_predicate(&mut sql, &mut args);
    if plan.cursor.is_some() {
        sql.push_str(" AND ");
        plan.push_cursor(&mut sql, &mut args, ACTIVITY_EXPR);
    }
    sql.push_str(&format!(
        " ORDER BY {ACTIVITY_EXPR} DESC, s.id DESC LIMIT ?) SELECT {ITEM_COLUMNS}, \
         {PREVIEW_SUBQUERY}, s.skill FROM page s ORDER BY {ACTIVITY_EXPR} DESC, s.id DESC"
    ));
    args.push(plan.limit.into());
    query_items(conn, &sql, args).await
}

/// Candidates from the covering index in keyset rounds, each resolved by a
/// point lookup without previews; unreadable rows are skipped and the rounds
/// advance until the page is full or the index is exhausted.
async fn list_via_point_lookups(
    conn: &Connection,
    plan: &ListPlan,
) -> Result<Vec<SessionListItem>> {
    let batch = plan.limit.saturating_mul(2).max(FALLBACK_BATCH_MIN);
    let mut cursor = plan.cursor.clone();
    let mut out = Vec::new();
    while out.len() < plan.limit as usize {
        let candidates = match next_candidates(conn, &cursor, batch).await {
            Ok(candidates) => candidates,
            Err(error) if is_unreadable_storage(&error) => {
                tracing::warn!(error = %error, "activity index unreadable; listing ends here");
                break;
            }
            Err(error) => return Err(error),
        };
        let Some(last) = candidates.last().cloned() else {
            break;
        };
        for (_, id) in candidates {
            match fetch_item(conn, plan, &id).await {
                Ok(Some(item)) => {
                    out.push(item);
                    if out.len() == plan.limit as usize {
                        return Ok(out);
                    }
                }
                Ok(None) => {}
                Err(error) if is_unreadable_storage(&error) => {
                    tracing::warn!(session_id = %id, error = %error, "skipping unreadable session row");
                }
                Err(error) => return Err(error),
            }
        }
        cursor = Some(last);
    }
    Ok(out)
}

/// `(activity, id)` pairs after `cursor`, newest first.
async fn next_candidates(
    conn: &Connection,
    cursor: &Option<(i64, String)>,
    batch: i64,
) -> Result<Vec<(i64, String)>> {
    let mut sql = format!(
        "SELECT {COVERING_ACTIVITY}, s.id FROM sessions s INDEXED BY idx_sessions_activity WHERE "
    );
    let mut args: Vec<Value> = Vec::new();
    if let Some((ts, id)) = cursor {
        sql.push_str(&format!(
            "({COVERING_ACTIVITY} < ? OR ({COVERING_ACTIVITY} = ? AND s.id < ?))"
        ));
        args.push((*ts).into());
        args.push((*ts).into());
        args.push(id.clone().into());
    } else {
        sql.push('1');
    }
    sql.push_str(&format!(
        " ORDER BY {COVERING_ACTIVITY} DESC, s.id DESC LIMIT ?"
    ));
    args.push(batch.into());
    let mut stmt = conn.prepare_cached(&sql).await?;
    let mut rows = stmt.query(params_from_iter(args)).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push((r.get::<i64>(0)?, r.get::<String>(1)?));
    }
    Ok(out)
}

/// `None` when the row exists but a row clause filters it out.
///
/// Deliberately without the preview subquery: previews walk the `messages`
/// b-tree, the largest table in the database and the one most likely to carry
/// damaged pages alongside the unreadable session row; a damaged interior
/// page there is not even reported as `Corrupt` by Turso 0.8.3 (its b-tree
/// seek asserts instead), which would take the whole listing down. A list
/// without previews beats no list; previews return once the database is
/// repaired and the fast path serves the page again.
async fn fetch_item(
    conn: &Connection,
    plan: &ListPlan,
    id: &str,
) -> Result<Option<SessionListItem>> {
    let mut sql =
        format!("SELECT {ITEM_COLUMNS}, NULL, s.skill FROM sessions s WHERE s.id = ? AND ");
    let mut args = vec![id.to_string().into()];
    plan.push_row_predicate(&mut sql, &mut args);
    Ok(query_items(conn, &sql, args).await?.into_iter().next())
}

async fn query_items(
    conn: &Connection,
    sql: &str,
    args: Vec<Value>,
) -> Result<Vec<SessionListItem>> {
    let mut stmt = conn.prepare_cached(sql).await?;
    let mut rows = stmt.query(params_from_iter(args)).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(SessionListItem {
            id: r.get::<String>(0).context("session id")?,
            kind: r.get::<Option<String>>(1)?,
            title: r.get::<Option<String>>(2)?,
            agent: normalize_agent(r.get::<Option<String>>(3)?),
            model: r.get::<Option<String>>(4)?,
            created_at: r.get::<i64>(5)?,
            updated_at: r.get::<i64>(6)?,
            preview: extract_preview(&r.get::<Option<String>>(7)?),
            skill: r.get::<Option<String>>(8)?,
        });
    }
    Ok(out)
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
