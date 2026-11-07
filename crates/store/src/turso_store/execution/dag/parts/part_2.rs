/// Collapse zombie runs of lost nodes: every `running`/`cancelling` run of a
/// node whose heartbeat is older than `stale_ms` becomes `error("node lost")`
/// stamped `finished_at`, with its synthetic `run_finished` frame appended in
/// the SAME transaction, in one transaction. Exclusive age bound (exactly
/// `stale_ms` old is still fresh), matching
/// [`super::node_tasks::converge_lost`]; `pending` rows never block claiming
/// and terminal rows are frozen, so the sweep is idempotent.
pub async fn converge_lost(
    conn: &Connection,
    now_ms: i64,
    stale_ms: i64,
) -> Result<Vec<ConvergedDagRun>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        // (a) Snapshot candidate ids: active runs of heartbeat-stale nodes
        // (unclaimed rows have NULL node_id and drop out of the JOIN).
        let ids = collect(
            conn,
            "SELECT dag_runs.id FROM dag_runs JOIN nodes ON dag_runs.node_id = nodes.id
             WHERE (?1 - nodes.last_seen_at) > ?2 AND dag_runs.status IN ('running','cancelling')",
            params![now_ms, stale_ms],
            |r| Ok(r.get::<String>(0)?),
        )
        .await?;
        // (b) Conditional CAS per row (no RETURNING, same as claim); each
        // winner also gets its in-transaction `run_finished` frame, so the
        // committed status can never outlive a missing final frame. The seq
        // is read as `max(seq)` under the held write lock.
        let mut out = Vec::new();
        for id in &ids {
            let changed = exec(
                conn,
                "UPDATE dag_runs SET status = 'error', error = 'node lost', finished_at = ?2
                 WHERE id = ?1 AND status IN ('running','cancelling')",
                params![id.as_str(), now_ms],
                "converge lost dag_run",
            )
            .await?;
            if changed == 0 {
                continue; // raced away between select and update
            }
            let run_finished_seq =
                insert_run_finished(conn, id, "error", Some("node lost"), now_ms).await?;
            if let Some(record) = get_run(conn, id).await? {
                out.push(ConvergedDagRun {
                    record,
                    run_finished_seq,
                });
            }
        }
        Ok(out)
    })
    .await
}

// ── helpers / row mapping ─────────────────────────────────────────────────

/// Append the synthetic `run_finished` frame for `run_id` and return its seq.
/// MUST run inside an open write transaction: the seq is recovered as
/// `max(seq)` under the held write lock, so no racing writer can interleave.
/// Payload matches what the persisted row would carry:
/// `{"status": ...}` + optional `"error"`.
async fn insert_run_finished(
    conn: &Connection,
    run_id: &str,
    status: &str,
    error: Option<&str>,
    at_ms: i64,
) -> Result<i64> {
    let mut payload = serde_json::json!({ "status": status });
    if let Some(err) = error {
        payload["error"] = serde_json::json!(err);
    }
    let payload_json = serde_json::to_string(&payload).context("serialize run_finished payload")?;
    exec(
        conn,
        super::dag_events::INSERT_EVENT,
        params![run_id, "run_finished", None::<&str>, payload_json, at_ms],
        "insert run_finished dag_event",
    )
    .await?;
    first(
        conn,
        "SELECT seq FROM dag_events ORDER BY seq DESC LIMIT 1",
        (),
        |r| Ok(r.get::<i64>(0)?),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("run_finished seq for dag run {run_id} vanished"))
}

/// `execute` + `context` — the shared write-half boilerplate.
async fn exec(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    ctx: &'static str,
) -> Result<u64> {
    conn.execute(sql, params).await.context(ctx)
}

/// Prepare + query + map every row — the shared read-half boilerplate
/// (also reused by [`super::dag_events`]).
pub(super) async fn collect<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: impl Fn(&turso::Row) -> Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare_cached(sql).await?;
    let mut rows = stmt.query(params).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(map(&r)?);
    }
    Ok(out)
}

/// [`collect`]'s LIMIT-1 sibling.
async fn first<T>(
    conn: &Connection,
    sql: &str,
    params: impl IntoParams,
    map: impl Fn(&turso::Row) -> Result<T>,
) -> Result<Option<T>> {
    let mut stmt = conn.prepare_cached(sql).await?;
    let mut rows = stmt.query(params).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(map(&r)?)),
        None => Ok(None),
    }
}

fn row_to_def(r: &turso::Row) -> Result<DagDefRecord> {
    Ok(DagDefRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        spec_json: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

fn row_to_run(r: &turso::Row) -> Result<DagRunRecord> {
    let s = r.get::<String>(5)?;
    Ok(DagRunRecord {
        id: r.get(0)?,
        dag_id: r.get(1)?,
        name: r.get(2)?,
        spec_json: r.get(3)?,
        node_id: r.get(4)?,
        status: DagRunStatus::parse(&s).with_context(|| format!("parse dag run status {s:?}"))?,
        error: r.get(6)?,
        created_at: r.get(7)?,
        claimed_at: r.get(8)?,
        finished_at: r.get(9)?,
    })
}
