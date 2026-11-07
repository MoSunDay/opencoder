// Transition writes, cancel polling, lost-node convergence and the
// session-cascading bulk deletes (dialog clears + delete_sessions).

/// Apply a validated transition atomically. Terminal writes stamp
/// `finished_at` and release a matching busy slot (`last_status='idle'`);
/// `last_task_id` stays put so UIs can keep showing recent work.
pub fn update_status(
    store: &rdb::store::Store,
    task_id: &str,
    to: NodeTaskStatus,
    error: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    let Some((row_key, mut task)) = load_by_id(store, task_id)? else {
        bail!("update_node_task_status: task {task_id} not found");
    };
    let from = task.status;
    if !node_state::transition_allowed(from, to) {
        bail!(
            "illegal node task transition {} -> {} for task {task_id}",
            from.as_str(),
            to.as_str()
        );
    }
    task.status = to;
    task.error = error.map(str::to_string);
    task.finished_at = if to.is_terminal() { Some(now_ms) } else { None };
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &row_key, &task)?;
    stage_release_busy(store, &mut batch, &task.node_id, task_id)?;
    tx::commit(store, batch)
}

/// Flag a pending/running task for cancellation; returns the previous status
/// when the flag landed, else `None` (idempotent, unknown = `None`).
pub fn request_cancel(store: &rdb::store::Store, task_id: &str) -> Result<Option<NodeTaskStatus>> {
    let Some((row_key, mut task)) = load_by_id(store, task_id)? else {
        return Ok(None);
    };
    match task.status {
        NodeTaskStatus::Pending | NodeTaskStatus::Running => {
            let prev = task.status;
            task.cancel_requested = true;
            task.status = NodeTaskStatus::Cancelling;
            let mut batch = rocksdb::WriteBatch::default();
            tx::put(&mut batch, &row_key, &task)?;
            tx::commit(store, batch)?;
            Ok(Some(prev))
        }
        _ => Ok(None),
    }
}

/// Release the node's busy slot exactly like terminal writes do
/// (`last_task_id == task_id` guard; 0-row UPDATE tolerated).
fn stage_release_busy(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    node_id: &str,
    task_id: &str,
) -> Result<()> {
    if let Some(mut node) = nodes::get(store, node_id)? {
        if node.last_task_id.as_deref() == Some(task_id) {
            node.last_status = "idle".to_string();
            tx::put(batch, &nodes::node_key(node_id), &node)?;
        }
    }
    Ok(())
}

/// Collapse zombie tasks of lost nodes: every `running`/`cancelling` task of
/// a node whose heartbeat is older than `stale_ms` (exclusive bound) becomes
/// `error("node lost")` stamped `finished_at`, releasing the busy slot.
/// `pending` rows are left alone; terminal rows are frozen (idempotent).
pub fn converge_lost(
    store: &rdb::store::Store,
    now_ms: i64,
    stale_ms: i64,
) -> Result<Vec<NodeTaskRecord>> {
    let mut last_seen = std::collections::HashMap::new();
    for (_, value) in tx::scan_prefix(store, b"node/")? {
        let node: crate::types::NodeRecord = kv::decode(&value)?;
        last_seen.insert(node.id.clone(), node.last_seen_at);
    }
    // (created_at, insertion ordinal) — a stable selection order.
    let mut candidates: Vec<(i64, i64, Vec<u8>, NodeTaskRecord)> = Vec::new();
    for (key, value) in tx::scan_prefix(store, b"ntask/")? {
        let Some((node_id, row_ord, _)) = parse_task_key(&key) else {
            continue;
        };
        let task: NodeTaskRecord = kv::decode(&value)?;
        if !matches!(
            task.status,
            NodeTaskStatus::Running | NodeTaskStatus::Cancelling
        ) {
            continue;
        }
        if last_seen
            .get(&node_id)
            .is_some_and(|seen| now_ms - seen > stale_ms)
        {
            candidates.push((task.created_at, row_ord, key, task));
        }
    }
    candidates.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let mut batch = rocksdb::WriteBatch::default();
    let mut out = Vec::new();
    for (_, _, row_key, mut task) in candidates {
        task.status = NodeTaskStatus::Error;
        task.error = Some("node lost".to_string());
        task.finished_at = Some(now_ms);
        tx::put(&mut batch, &row_key, &task)?;
        stage_release_busy(store, &mut batch, &task.node_id, &task.id)?;
        out.push(task);
    }
    tx::commit(store, batch)?;
    Ok(out)
}

/// Bulk-clear a node's console dialogs: delete the sessions of every
/// TERMINAL task (done | error | cancelled) of `node_id`; sessions of
/// pending/running/cancelling tasks are kept and reported as `skipped`
/// (`created_at DESC`, like the SQL listing).
pub fn clear_finished_sessions(
    store: &rdb::store::Store,
    node_id: &str,
) -> Result<crate::types::ClearNodeDialogs> {
    let mut skipped: Vec<(i64, String)> = Vec::new();
    let mut terminal: Vec<NodeTaskRecord> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &nodes::node_task_prefix(node_id))? {
        if parse_task_key(&key).is_none() {
            continue;
        }
        let task: NodeTaskRecord = kv::decode(&value)?;
        if task.status.is_terminal() {
            terminal.push(task);
        } else {
            skipped.push((task.created_at, task.session_id));
        }
    }
    skipped.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let skipped: Vec<String> = skipped.into_iter().map(|(_, id)| id).collect();

    let mut batch = rocksdb::WriteBatch::default();
    let mut removed = 0u64;
    for task in &terminal {
        if conversation::sessions::get(store, &task.session_id)?.is_some() {
            removed += 1;
        }
        stage_session_task_delete(store, &mut batch, &task.session_id)?;
    }
    tx::commit(store, batch)?;
    Ok(crate::types::ClearNodeDialogs { removed, skipped })
}

/// `delete_sessions(ids)`: remove each EXISTING session with all children
/// (messages, events, inputs, subagent tasks, todo workflows) plus any node
/// task bound to it (`session_id` FK cascade) in ONE batch; returns the
/// number of sessions that actually existed.
pub fn delete_sessions(store: &rdb::store::Store, ids: &[String]) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut batch = rocksdb::WriteBatch::default();
    let mut removed = 0u64;
    for id in ids {
        if conversation::sessions::get(store, id)?.is_some() {
            removed += 1;
            stage_session_task_delete(store, &mut batch, id)?;
        }
    }
    tx::commit(store, batch)?;
    Ok(removed)
}

/// Stage a full session delete (row + all child families + any bound node
/// task row with its indexes) inside the caller's batch — the KV form of the
/// schema's `ON DELETE CASCADE` chain.
pub(crate) fn stage_session_task_delete(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    session_id: &str,
) -> Result<()> {
    batch.delete(kv::session_key(session_id));
    tx::delete_prefix(store, batch, &kv::message_prefix(session_id))?;
    tx::delete_prefix(store, batch, &kv::event_prefix(session_id))?;
    conversation::sessions::delete_input_children(store, batch, session_id)?;
    conversation::subagent_tasks::delete_for_session(store, batch, session_id)?;
    todo::workflows::delete_for_session(store, batch, session_id)?;
    if let Some(row_key) = index::owner(store, &session_index(session_id))? {
        if let Some(value) = rdb::store::get(store, b"", &row_key).map_err(kv::engine_err)? {
            if let Ok(task) = kv::decode::<NodeTaskRecord>(&value) {
                tx::del(batch, &task_id_index(&task.id));
            }
            tx::del(batch, &row_key);
        }
        tx::del(batch, &session_index(session_id));
    }
    Ok(())
}
