// Read-half queries: per-node listing, fleet-wide filtered listing (FIFO),
// id and synthetic-session lookups.

/// Load a task through its id index (dangling index entries read as absent).
fn load_by_id(
    store: &rdb::store::Store,
    task_id: &str,
) -> Result<Option<(Vec<u8>, NodeTaskRecord)>> {
    let Some(row_key) = live_owner(store, &task_id_index(task_id))? else {
        return Ok(None);
    };
    let Some(value) = rdb::store::get(store, b"", &row_key).map_err(kv::engine_err)? else {
        return Ok(None);
    };
    Ok(Some((row_key, kv::decode(&value)?)))
}

pub fn get_task(store: &rdb::store::Store, task_id: &str) -> Result<Option<NodeTaskRecord>> {
    Ok(load_by_id(store, task_id)?.map(|(_, task)| task))
}

/// Reverse lookup by the task's unique synthetic session.
pub fn get_by_session(
    store: &rdb::store::Store,
    session_id: &str,
) -> Result<Option<NodeTaskRecord>> {
    let Some(row_key) = live_owner(store, &session_index(session_id))? else {
        return Ok(None);
    };
    match rdb::store::get(store, b"", &row_key).map_err(kv::engine_err)? {
        Some(value) => Ok(Some(kv::decode(&value)?)),
        None => Ok(None),
    }
}

/// All tasks of one node, newest `created_at` first (insertion ordinal on
/// ties), bounded by `limit`.
pub fn list_tasks(
    store: &rdb::store::Store,
    node_id: &str,
    limit: u32,
) -> Result<Vec<NodeTaskRecord>> {
    let mut rows: Vec<(i64, NodeTaskRecord)> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &nodes::node_task_prefix(node_id))? {
        let Some((_, row_ord, _)) = parse_task_key(&key) else {
            continue;
        };
        rows.push((row_ord, kv::decode(&value)?));
    }
    rows.sort_by(|a, b| (&a.1.created_at, &a.0).cmp(&(&b.1.created_at, &b.0)));
    let mut out: Vec<NodeTaskRecord> = rows.into_iter().map(|(_, t)| t).collect();
    out.reverse(); // created_at DESC
    out.truncate(limit as usize);
    Ok(out)
}

/// Fleet-wide listing with optional `node_id` / `status` filters, FIFO order
/// (`created_at ASC`, insertion ordinal ASC) — the exact drain order of
/// [`super::claim_next`], same-ms ties included.
pub fn list_tasks_filtered(
    store: &rdb::store::Store,
    node_id: Option<&str>,
    status: Option<NodeTaskStatus>,
    limit: u32,
) -> Result<Vec<NodeTaskRecord>> {
    let mut rows: Vec<(i64, NodeTaskRecord)> = Vec::new();
    for (key, value) in tx::scan_prefix(store, b"ntask/")? {
        let Some((task_node, row_ord, _)) = parse_task_key(&key) else {
            continue;
        };
        if node_id.is_some_and(|n| task_node != n) {
            continue;
        }
        let task: NodeTaskRecord = kv::decode(&value)?;
        if status.is_some_and(|s| task.status != s) {
            continue;
        }
        rows.push((row_ord, task));
    }
    rows.sort_by(|a, b| (&a.1.created_at, &a.0).cmp(&(&b.1.created_at, &b.0)));
    rows.truncate(limit.max(1) as usize);
    Ok(rows.into_iter().map(|(_, t)| t).collect())
}
