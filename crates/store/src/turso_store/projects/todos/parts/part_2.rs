fn bounded_json(
    value: Option<String>,
    bytes: Option<i64>,
    field: &str,
) -> Result<serde_json::Value> {
    match (value, bytes) {
        (_, None) => Ok(serde_json::Value::Null),
        (Some(value), Some(bytes)) if bytes <= 64 * 1024 => Ok(serde_json::from_str(&value)?),
        (_, Some(bytes)) => Ok(serde_json::json!({
            "omitted": true,
            "total_bytes": bytes.max(0),
            "read_via": "detail_field",
            "field": field,
        })),
    }
}

fn bounded_text(value: Option<String>, bytes: Option<i64>, field: &str) -> serde_json::Value {
    match (value, bytes) {
        (_, None) => serde_json::Value::Null,
        (Some(value), Some(bytes)) if bytes <= 64 * 1024 => serde_json::Value::String(value),
        (_, Some(bytes)) => serde_json::json!({
            "omitted": true,
            "total_bytes": bytes.max(0),
            "read_via": "detail_field",
            "field": field,
        }),
    }
}

pub async fn events_after(
    conn: &Connection,
    workflow_id: &str,
    after: i64,
) -> Result<Vec<TodoEventRecord>> {
    let mut rows = conn
        .query(
            "SELECT seq,workflow_id,kind,payload_json,ts FROM todo_events WHERE workflow_id=? AND seq>? ORDER BY seq",
            params![workflow_id, after],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(TodoEventRecord {
            seq: Some(row.get(0)?),
            workflow_id: row.get(1)?,
            kind: row.get(2)?,
            payload: serde_json::from_str(&row.get::<String>(3)?)?,
            ts: row.get(4)?,
        });
    }
    Ok(out)
}

pub async fn events_page(
    conn: &Connection,
    workflow_id: &str,
    after: i64,
    limit: u32,
    payload_budget: usize,
) -> Result<crate::TodoEventPage> {
    bounded_events(conn, workflow_id, after, limit, payload_budget, false).await
}

pub async fn events_before(
    conn: &Connection,
    workflow_id: &str,
    before: i64,
    limit: u32,
    payload_budget: usize,
) -> Result<crate::TodoEventPage> {
    bounded_events(conn, workflow_id, before, limit, payload_budget, true).await
}

async fn bounded_events(
    conn: &Connection,
    workflow_id: &str,
    cursor: i64,
    limit: u32,
    payload_budget: usize,
    reverse: bool,
) -> Result<crate::TodoEventPage> {
    let limit = limit.clamp(1, 200) as usize;
    let mut rows = conn
        .query(
            if reverse {
                "SELECT seq,kind,ts,length(CAST(payload_json AS BLOB)) FROM todo_events \
                 WHERE workflow_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT ?3"
            } else {
                "SELECT seq,kind,ts,length(CAST(payload_json AS BLOB)) FROM todo_events \
                 WHERE workflow_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3"
            },
            params![workflow_id, cursor, limit as i64 + 1],
        )
        .await?;
    let mut metas = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().await? {
        metas.push((
            row.get::<i64>(0)?,
            row.get::<String>(1)?,
            row.get::<i64>(2)?,
            row.get::<i64>(3)?.max(0) as usize,
        ));
    }
    drop(rows);
    let mut events = Vec::with_capacity(limit.min(metas.len()));
    let mut used = 0usize;
    for (seq, kind, ts, payload_len) in metas.iter().take(limit) {
        let bytes = payload_len.saturating_add(512);
        if bytes > payload_budget && events.is_empty() {
            events.push(TodoEventRecord {
                seq: Some(*seq),
                workflow_id: workflow_id.into(),
                kind: kind.clone(),
                payload: serde_json::json!({
                    "omitted": true,
                    "reason": "event_payload_exceeds_page_budget",
                    "total_bytes": payload_len,
                    "read_via": "event_payload",
                }),
                ts: *ts,
            });
            break;
        }
        if used.saturating_add(bytes) > payload_budget {
            break;
        }
        let mut payload = conn
            .query(
                "SELECT payload_json FROM todo_events WHERE workflow_id=?1 AND seq=?2",
                params![workflow_id, *seq],
            )
            .await?;
        let Some(row) = payload.next().await? else {
            anyhow::bail!("todo event {seq} disappeared during pagination");
        };
        events.push(TodoEventRecord {
            seq: Some(*seq),
            workflow_id: workflow_id.into(),
            kind: kind.clone(),
            payload: serde_json::from_str(&row.get::<String>(0)?)?,
            ts: *ts,
        });
        used += bytes;
    }
    Ok(crate::TodoEventPage {
        more: events.len() < metas.len(),
        events,
    })
}

pub async fn event_payload_chunk(
    conn: &Connection,
    workflow_id: &str,
    seq: i64,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let start = i64::try_from(offset)?.saturating_add(1);
    let mut rows = conn
        .query(
            "SELECT length(CAST(payload_json AS BLOB)), \
             CAST(substr(CAST(payload_json AS BLOB),?3,?4) AS BLOB) \
             FROM todo_events WHERE workflow_id=?1 AND seq=?2",
            params![workflow_id, seq, start, take],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let total = row.get::<i64>(0)?.max(0) as u64;
    if offset > total {
        anyhow::bail!("todo event payload offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get(1)?,
    }))
}
