//! Todo event projections (`tev/<workflow_id>/<seq:020>`).

use crate::rdb_store::kv::{get_json, todo_event_key, todo_event_prefix};
use crate::rdb_store::tx;
use crate::{TodoEventPage, TodoEventRecord};

/// Allocate the next per-workflow event seq and stage the row.
pub(crate) fn stage_append(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    event: &TodoEventRecord,
) -> anyhow::Result<i64> {
    let seq = crate::rdb_store::seq::next_child(store, &todo_event_prefix(&event.workflow_id));
    let record = TodoEventRecord {
        seq: Some(seq),
        workflow_id: event.workflow_id.clone(),
        kind: event.kind.clone(),
        payload: event.payload.clone(),
        ts: event.ts,
    };
    tx::put(batch, &todo_event_key(&event.workflow_id, seq), &record)?;
    Ok(seq)
}

/// Highest event seq of a workflow (0 when the workflow has no events).
pub fn last_seq(store: &rdb::store::Store, workflow_id: &str) -> i64 {
    crate::rdb_store::seq::max_child(store, &todo_event_prefix(workflow_id))
}

/// Events with `seq > after` in ascending order.
pub fn after(
    store: &rdb::store::Store,
    workflow_id: &str,
    after_seq: i64,
) -> anyhow::Result<Vec<TodoEventRecord>> {
    let mut out = Vec::new();
    for (key, value) in tx::scan_prefix(store, &todo_event_prefix(workflow_id))? {
        if let Some(seq) = crate::rdb_store::seq::parse_ord(&key, &todo_event_prefix(workflow_id)) {
            if seq > after_seq {
                out.push(crate::rdb_store::kv::decode(&value)?);
            }
        }
    }
    Ok(out)
}

/// Bounded event page (forward `seq > after` or reverse `seq < before`).
pub fn page(
    store: &rdb::store::Store,
    workflow_id: &str,
    cursor: i64,
    limit: u32,
    payload_budget: usize,
    reverse: bool,
) -> anyhow::Result<TodoEventPage> {
    let limit = limit.clamp(1, 200) as usize;
    let mut metas = Vec::new();
    for (key, value) in tx::scan_prefix(store, &todo_event_prefix(workflow_id))? {
        let Some(seq) = crate::rdb_store::seq::parse_ord(&key, &todo_event_prefix(workflow_id))
        else {
            continue;
        };
        let record: TodoEventRecord = crate::rdb_store::kv::decode(&value)?;
        let payload_len = serde_json::to_vec(&record.payload)?.len();
        let keep = if reverse { seq < cursor } else { seq > cursor };
        if keep {
            metas.push((seq, record, payload_len));
        }
    }
    if reverse {
        metas.reverse(); // seq DESC
    }
    metas.truncate(limit + 1);

    let mut events = Vec::new();
    let mut used = 0usize;
    for (seq, record, payload_len) in metas.iter().take(limit) {
        let bytes = payload_len.saturating_add(512);
        if bytes > payload_budget && events.is_empty() {
            events.push(TodoEventRecord {
                seq: Some(*seq),
                workflow_id: workflow_id.to_string(),
                kind: record.kind.clone(),
                payload: serde_json::json!({
                    "omitted": true,
                    "reason": "event_payload_exceeds_page_budget",
                    "total_bytes": payload_len,
                    "read_via": "event_payload",
                }),
                ts: record.ts,
            });
            break;
        }
        if used.saturating_add(bytes) > payload_budget {
            break;
        }
        events.push(record.clone());
        used += bytes;
    }
    Ok(TodoEventPage {
        more: events.len() < metas.len(),
        events,
    })
}

/// Chunked payload read (SQL `substr` over the stored payload JSON).
pub fn payload_chunk(
    store: &rdb::store::Store,
    workflow_id: &str,
    seq: i64,
    offset: u64,
    max_bytes: usize,
) -> anyhow::Result<Option<crate::PayloadChunkRecord>> {
    let Some(record) = get_json::<TodoEventRecord>(store, &todo_event_key(workflow_id, seq))?
    else {
        return Ok(None);
    };
    let payload = serde_json::to_vec(&record.payload)?;
    let total = payload.len() as u64;
    if offset > total {
        anyhow::bail!("todo event payload offset exceeds total bytes");
    }
    let take = max_bytes.clamp(1, 64 * 1024);
    let start = offset as usize;
    let end = start.saturating_add(take).min(payload.len());
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: payload[start..end].to_vec(),
    }))
}
