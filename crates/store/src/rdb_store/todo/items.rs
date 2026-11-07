//! Todo item projections (`tit/<workflow_id>/<todo_id>`), ordered by ordinal.

use crate::rdb_store::kv::{get_json, todo_item_key, todo_item_prefix};
use crate::rdb_store::tx;
use crate::{TodoItemPage, TodoItemRecord, TodoItemSummary};

/// All items of a workflow in ordinal order (`ORDER BY ordinal`).
pub fn items(store: &rdb::store::Store, workflow_id: &str) -> anyhow::Result<Vec<TodoItemRecord>> {
    scan(store, workflow_id)
}

fn scan(store: &rdb::store::Store, workflow_id: &str) -> anyhow::Result<Vec<TodoItemRecord>> {
    let mut rows = Vec::new();
    for (_key, value) in tx::scan_prefix(store, &todo_item_prefix(workflow_id))? {
        rows.push(crate::rdb_store::kv::decode::<TodoItemRecord>(&value)?);
    }
    rows.sort_by_key(|r| (r.ordinal, r.todo_id.clone()));
    Ok(rows)
}

/// Upsert-only item projection (INSERT OR REPLACE on the natural key).
pub(crate) fn stage_upsert(batch: &mut rocksdb::WriteBatch, items: &[TodoItemRecord]) {
    for item in items {
        let _ = tx::put(
            batch,
            &todo_item_key(&item.workflow_id, &item.todo_id),
            item,
        );
    }
}

/// Keyset-paginated inspection rows; oversized fields become markers.
pub fn items_page(
    store: &rdb::store::Store,
    workflow_id: &str,
    after_ordinal: Option<i64>,
    limit: u32,
) -> anyhow::Result<TodoItemPage> {
    let limit = limit.clamp(1, 100) as usize;
    let all = scan(store, workflow_id)?;
    let mut items = Vec::with_capacity(limit);
    let mut overflow = false;
    for item in all {
        if let Some(after) = after_ordinal {
            if item.ordinal <= after {
                continue;
            }
        }
        if items.len() == limit {
            // Cursor = last INCLUDED ordinal (strictly-after semantics),
            // mirroring the SQL `LIMIT n+1` keyset pattern exactly.
            overflow = true;
            break;
        }
        items.push(TodoItemSummary {
            workflow_id: item.workflow_id.clone(),
            todo_id: item.todo_id.clone(),
            ordinal: item.ordinal,
            status: item.status,
            attempt: item.attempt,
            active_session_id: item.active_session_id.clone(),
            session_history: bounded_json(
                &serde_json::to_vec(&item.session_history)?,
                &format!("todo.item.{}.session_history", item.todo_id),
            ),
            result_json: match &item.result_json {
                None => serde_json::Value::Null,
                Some(value) => bounded_json(
                    &serde_json::to_vec(value)?,
                    &format!("todo.item.{}.result_json", item.todo_id),
                ),
            },
            last_error: match &item.last_error {
                None => serde_json::Value::Null,
                Some(text) => bounded_json(
                    text.as_bytes(),
                    &format!("todo.item.{}.last_error", item.todo_id),
                ),
            },
            updated_at: item.updated_at,
        });
    }
    let next_ordinal = overflow.then(|| items.last().expect("limit>0 guarantees an item").ordinal);
    Ok(TodoItemPage {
        items,
        next_ordinal,
    })
}

const BOUND: usize = 64 * 1024;

fn bounded_json(bytes: &[u8], field: &str) -> serde_json::Value {
    if bytes.len() <= BOUND {
        if let Ok(value) = serde_json::from_slice(bytes) {
            return value;
        }
    }
    serde_json::json!({
        "omitted": true,
        "total_bytes": bytes.len(),
        "read_via": "detail_field",
        "field": field,
    })
}

/// Chunked read of one item field (SQL `substr` over the stored column).
pub fn field_chunk(
    store: &rdb::store::Store,
    workflow_id: &str,
    todo_id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> anyhow::Result<Option<crate::PayloadChunkRecord>> {
    let Some(item) = get_json::<TodoItemRecord>(store, &todo_item_key(workflow_id, todo_id))?
    else {
        return Ok(None);
    };
    let bytes: Vec<u8> = match field {
        "session_history" => serde_json::to_vec(&item.session_history)?,
        "result_json" => match item.result_json {
            Some(ref value) => serde_json::to_vec(value)?,
            None => return Ok(None),
        },
        "last_error" => match item.last_error {
            Some(ref text) => text.clone().into_bytes(),
            None => return Ok(None),
        },
        _ => anyhow::bail!("unsupported todo item field"),
    };
    let total = bytes.len() as u64;
    if offset > total {
        anyhow::bail!("todo item field offset exceeds total bytes");
    }
    let take = max_bytes.clamp(1, 64 * 1024);
    let start = offset as usize;
    let end = start.saturating_add(take).min(bytes.len());
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: bytes[start..end].to_vec(),
    }))
}
