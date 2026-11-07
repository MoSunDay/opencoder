//! Session listing: activity ordering, keyset cursor, filters, preview.
//!
//! Ports the SQL listing semantics: order by `MAX(updated_at, created_at)`
//! DESC then id DESC; cursor `{activity_ts}|{id}`; `workdir_hash` equality;
//! escaped substring search over id/title (ASCII case-insensitive, like
//! SQLite `LIKE`); task-type visibility fences; kind lane fence; preview =
//! first user message's first text block capped at 80 chars.

use anyhow::{anyhow, Result};

use super::sessions::{self, SessionRow};
use crate::rdb_store::{kv, tx};
use crate::types::{
    SessionFilter, SessionListItem, TASK_TYPE_AGENT_STEP, TASK_TYPE_PARENT, TASK_TYPE_SUBAGENT,
};

/// SQLite `LIKE` is ASCII case-insensitive; mirror it for the search filter.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    let lower = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c
                }
            })
            .collect()
    };
    lower(haystack).contains(&lower(needle))
}

/// `MAX(updated_at, created_at)` — the activity sort key.
fn activity(row: &SessionRow) -> i64 {
    row.meta.updated_at.max(row.meta.created_at)
}

fn decode_cursor(c: &str) -> Option<(i64, String)> {
    let mut it = c.splitn(2, '|');
    let ts: i64 = it.next()?.parse().ok()?;
    let id = it.next()?.to_string();
    Some((ts, id))
}

/// `normalize_agent`: the legacy `sandbox` agent maps to `plan`.
fn normalize_agent(agent: Option<String>) -> Option<String> {
    agent.map(|a| {
        if a == "sandbox" {
            "plan".to_string()
        } else {
            a
        }
    })
}

/// Does any subagent task reference `session_id` as its child?
fn has_subagent_child(store: &rdb::store::Store, session_id: &str) -> bool {
    crate::rdb_store::index::any_under(store, &kv::subagent_child_prefix(session_id))
}

fn visible(
    store: &rdb::store::Store,
    row: &SessionRow,
    include_subagents: bool,
    include_execution: bool,
) -> Result<bool> {
    let tt = row.meta.task_type.as_deref().unwrap_or(TASK_TYPE_PARENT);
    if include_execution {
        // Top-level parents (minus subagent children) + Agent-step detail.
        return Ok(tt == TASK_TYPE_AGENT_STEP
            || (tt == TASK_TYPE_PARENT && !has_subagent_child(store, &row.meta.id)));
    }
    if include_subagents {
        Ok(tt == TASK_TYPE_PARENT || tt == TASK_TYPE_SUBAGENT)
    } else {
        Ok(tt == TASK_TYPE_AGENT_STEP
            || (tt == TASK_TYPE_PARENT && !has_subagent_child(store, &row.meta.id)))
    }
}

pub fn list(
    store: &rdb::store::Store,
    filter: &SessionFilter,
    include_execution: bool,
) -> Result<Vec<SessionListItem>> {
    let limit = filter.limit.clamp(1, 500) as usize;
    let (cursor_ts, cursor_id) = match &filter.cursor {
        Some(c) => decode_cursor(c).ok_or_else(|| anyhow!("invalid session list cursor"))?,
        None => (i64::MAX, String::new()),
    };
    let mut rows: Vec<SessionRow> = sessions::all_rows(store)?
        .into_iter()
        .filter(|r| match &filter.workdir_hash {
            Some(h) => r.meta.workdir_hash.as_deref() == Some(h.as_str()),
            None => true,
        })
        .filter(|r| match &filter.search {
            Some(s) => {
                contains_ci(&r.meta.id, s)
                    || r.meta.title.as_deref().is_some_and(|t| contains_ci(t, s))
            }
            None => true,
        })
        .filter(|r| match &filter.kind {
            Some(kind) => r.meta.kind.as_deref() == Some(kind.as_str()),
            None => r.meta.kind.as_deref() != Some("operator"),
        })
        .filter(|r| visible(store, r, filter.include_subagents, include_execution).unwrap_or(false))
        .filter(|r| {
            // Keyset cursor: strictly below (activity, id) in DESC order.
            let a = activity(r);
            a < cursor_ts || (a == cursor_ts && r.meta.id < cursor_id)
        })
        .collect();
    rows.sort_by(|a, b| {
        activity(b)
            .cmp(&activity(a))
            .then_with(|| b.meta.id.cmp(&a.meta.id))
    });
    rows.truncate(limit);
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(SessionListItem {
            preview: preview(store, &r.meta.id)?,
            id: r.meta.id.clone(),
            kind: r.meta.kind.clone(),
            title: r.meta.title.clone(),
            agent: normalize_agent(r.meta.agent.clone()),
            model: r.meta.model.clone(),
            created_at: r.meta.created_at,
            updated_at: r.meta.updated_at,
            skill: r.meta.skill.clone(),
        });
    }
    Ok(out)
}

/// First user message's first text block, capped at 80 chars.
pub fn preview(store: &rdb::store::Store, session_id: &str) -> Result<String> {
    let prefix = kv::message_prefix(session_id);
    for (_, value) in tx::scan_prefix(store, &prefix)? {
        let rec: super::messages::MessageRecord = kv::decode(&value)?;
        if rec.role != "user" {
            continue;
        }
        // The SQL listing read at most 8 KiB of `blocks_json`; an oversized
        // (or NUL-bearing) payload truncated mid-JSON and degraded to an
        // empty preview instead of materializing the block.
        let blocks_json = serde_json::to_vec(&rec.blocks)?;
        if blocks_json.len() > 8192 || blocks_json.contains(&0) {
            return Ok(String::new());
        }
        for block in &rec.blocks {
            if let opencoder_core::ContentBlock::Text { text } = block {
                return Ok(text.chars().take(80).collect());
            }
        }
        return Ok(String::new());
    }
    Ok(String::new())
}
