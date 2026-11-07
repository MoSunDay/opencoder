// Run state machine: transitions, atomic finalize (+ synthetic run_finished
// frame), cancel, cancel piggyback listing, reads and the lost-node sweep.

/// Load a run through its id index.
fn load_by_id(store: &rdb::store::Store, run_id: &str) -> Result<Option<(i64, DagRunRecord)>> {
    let Some(row_key) = index::owner(store, &run_id_index(run_id))? else {
        return Ok(None);
    };
    let Some(value) = rdb::store::get(store, b"", &row_key).map_err(kv::engine_err)? else {
        return Ok(None);
    };
    let row_ord = parse_run_key(&row_key).map_or(0, |(ord, _)| ord);
    Ok(Some((row_ord, kv::decode(&value)?)))
}

pub fn get_run(store: &rdb::store::Store, run_id: &str) -> Result<Option<DagRunRecord>> {
    Ok(load_by_id(store, run_id)?.map(|(_, r)| r))
}

/// Newest-first listing (`created_at DESC, insertion DESC`) — the exact
/// reverse of the claim FIFO order.
pub fn list_runs(store: &rdb::store::Store, limit: u32) -> Result<Vec<DagRunRecord>> {
    let mut rows = runs_fifo(store)?;
    rows.reverse();
    let mut out: Vec<DagRunRecord> = rows.into_iter().map(|(_, r)| r).collect();
    out.truncate(limit as usize);
    Ok(out)
}

/// Apply a validated transition atomically; terminal writes stamp
/// `finished_at` (non-terminal writes clear it). The grid rejects every
/// same-state no-op (terminal states freeze, live states never self-loop).
pub fn update_status(
    store: &rdb::store::Store,
    run_id: &str,
    to: DagRunStatus,
    error: Option<&str>,
    now_ms: i64,
) -> Result<DagRunRecord> {
    let Some((row_ord, mut run)) = load_by_id(store, run_id)? else {
        bail!("update_dag_run_status: dag run {run_id} not found");
    };
    let from = run.status;
    if !transition_allowed(from, to) {
        bail!("illegal dag run transition {from} -> {to} for run {run_id}");
    }
    run.status = to;
    run.error = error.map(str::to_string);
    run.finished_at = if to.is_terminal() { Some(now_ms) } else { None };
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &run_row_key(row_ord, run_id), &run)?;
    tx::commit(store, batch)?;
    Ok(run)
}

/// Terminal status move + synthetic `run_finished` event in ONE batch: a
/// crash between the status flip and the frame append is impossible, so a
/// terminal run is never left without its final frame. Returns the assigned
/// event `seq`. Same error contract as [`update_status`].
pub fn finalize_run(
    store: &rdb::store::Store,
    run_id: &str,
    status: DagRunStatus,
    error: Option<&str>,
    now_ms: i64,
) -> Result<i64> {
    let Some((row_ord, mut run)) = load_by_id(store, run_id)? else {
        bail!("finalize_dag_run: dag run {run_id} not found");
    };
    let from = run.status;
    if !transition_allowed(from, status) {
        bail!("illegal dag run transition {from} -> {status} for run {run_id}");
    }
    run.status = status;
    run.error = error.map(str::to_string);
    run.finished_at = if status.is_terminal() {
        Some(now_ms)
    } else {
        None
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &run_row_key(row_ord, run_id), &run)?;
    let mut carry = crate::rdb_store::seq::SeqCarry::new();
    let seq = super::dag_events::stage_run_finished(
        store,
        &mut batch,
        &mut carry,
        run_id,
        status.as_str(),
        error,
        now_ms,
    )?;
    tx::commit(store, batch)?;
    Ok(seq)
}

/// Request cancellation: `pending` collapses straight to `cancelled`
/// (nothing claimed it yet), `running` flips to `cancelling` for the node to
/// observe via the heartbeat piggyback, `cancelling` is an idempotent no-op,
/// terminal states bail.
pub fn cancel(store: &rdb::store::Store, run_id: &str, now_ms: i64) -> Result<()> {
    let Some((row_ord, mut run)) = load_by_id(store, run_id)? else {
        bail!("cancel_dag_run: dag run {run_id} not found");
    };
    match run.status {
        DagRunStatus::Pending => {
            run.status = DagRunStatus::Cancelled;
            run.finished_at = Some(now_ms);
        }
        DagRunStatus::Running => {
            run.status = DagRunStatus::Cancelling;
        }
        DagRunStatus::Cancelling => return Ok(()), // already requested
        terminal => {
            bail!("cancel_dag_run: dag run {run_id} already terminal ({terminal})")
        }
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &run_row_key(row_ord, run_id), &run)?;
    tx::commit(store, batch)
}

/// `cancelling` runs of `node_id` — the heartbeat piggyback payload
/// (`created_at ASC`, insertion ordinal on ties).
pub fn cancelling_runs(store: &rdb::store::Store, node_id: &str) -> Result<Vec<String>> {
    Ok(runs_fifo(store)?
        .into_iter()
        .filter(|(_, r)| {
            r.status == DagRunStatus::Cancelling && r.node_id.as_deref() == Some(node_id)
        })
        .map(|(_, r)| r.id)
        .collect())
}

/// Collapse zombie runs of lost nodes: every `running`/`cancelling` run of a
/// node whose heartbeat is older than `stale_ms` (exclusive bound) becomes
/// `error("node lost")` stamped `finished_at`, with its synthetic
/// `run_finished` frame appended in the SAME batch. Unclaimed rows have no
/// node and drop out; `pending` rows never block claiming and terminal rows
/// are frozen, so the sweep is idempotent.
pub fn converge_lost(
    store: &rdb::store::Store,
    now_ms: i64,
    stale_ms: i64,
) -> Result<Vec<ConvergedDagRun>> {
    let mut last_seen = std::collections::HashMap::new();
    for (_, value) in tx::scan_prefix(store, b"node/")? {
        let node: crate::types::NodeRecord = kv::decode(&value)?;
        last_seen.insert(node.id.clone(), node.last_seen_at);
    }
    let mut out = Vec::new();
    let mut batch = rocksdb::WriteBatch::default();
    let mut carry = crate::rdb_store::seq::SeqCarry::new();
    for (row_ord, mut run) in runs_fifo(store)? {
        if !matches!(run.status, DagRunStatus::Running | DagRunStatus::Cancelling) {
            continue;
        }
        let Some(node_id) = run.node_id.clone() else {
            continue;
        };
        if !last_seen
            .get(&node_id)
            .is_some_and(|seen| now_ms - seen > stale_ms)
        {
            continue;
        }
        run.status = DagRunStatus::Error;
        run.error = Some("node lost".to_string());
        run.finished_at = Some(now_ms);
        tx::put(&mut batch, &run_row_key(row_ord, &run.id), &run)?;
        let seq = super::dag_events::stage_run_finished(
            store,
            &mut batch,
            &mut carry,
            &run.id,
            "error",
            Some("node lost"),
            now_ms,
        )?;
        out.push(ConvergedDagRun {
            record: run,
            run_finished_seq: seq,
        });
    }
    tx::commit(store, batch)?;
    Ok(out)
}
