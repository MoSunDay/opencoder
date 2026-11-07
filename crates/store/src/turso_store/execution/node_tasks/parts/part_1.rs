const TASK_COLS: &str = "id, node_id, session_id, title, prompt, agent, model, status, error, cancel_requested, created_at, claimed_at, finished_at";

/// Enqueue a node task in one transaction: verify the node exists, insert
/// the synthetic session (`task_type="node"`), then queue it as `pending`.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
    node_id: &str,
    title: Option<&str>,
    prompt: &str,
    agent: Option<&str>,
    model: Option<&str>,
    now_ms: i64,
) -> Result<NodeTaskRecord> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        if !node_exists(conn, node_id).await? {
            anyhow::bail!("dispatch_node_task: node {node_id} does not exist");
        }
        // Session first: node_tasks.session_id is an immediate FK into it.
        super::sessions::create(
            conn,
            &SessionMeta {
                kind: None,
                id: session_id.to_string(),
                title: title.map(str::to_string),
                agent: agent.map(str::to_string),
                model: model.map(str::to_string),
                autopilot_mode: None,
                workdir_hash: None,
                created_at: now_ms,
                updated_at: now_ms,
                summary: None,
                summary_seq: None,
                summary_images: Vec::new(),
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: Some(TASK_TYPE_NODE.to_string()),
                requirement: None,
            },
        )
        .await?;
        insert_task_row(
            conn, task_id, session_id, node_id, title, prompt, agent, model, now_ms,
        )
        .await?;
        get_task(conn, task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("dispatch_node_task: row {task_id} vanished"))
    })
    .await
}

/// Enqueue a node task bound to an EXISTING session (dialog resume): the
/// session row is reused untouched — only the `node_tasks` row appears. The
/// in-transaction existence check keeps the FK honest even under a racing
/// delete; the HTTP layer maps the error to 400.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_for_session(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
    node_id: &str,
    title: Option<&str>,
    prompt: &str,
    agent: Option<&str>,
    model: Option<&str>,
    now_ms: i64,
) -> Result<NodeTaskRecord> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        if !node_exists(conn, node_id).await? {
            anyhow::bail!("dispatch_node_task: node {node_id} does not exist");
        }
        if !session_exists(conn, session_id).await? {
            anyhow::bail!("dispatch_node_task: session {session_id} does not exist");
        }
        insert_task_row(
            conn, task_id, session_id, node_id, title, prompt, agent, model, now_ms,
        )
        .await?;
        get_task(conn, task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("dispatch_node_task: row {task_id} vanished"))
    })
    .await
}

/// Shared `INSERT INTO node_tasks` (pending, unclaimed).
#[allow(clippy::too_many_arguments)]
async fn insert_task_row(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
    node_id: &str,
    title: Option<&str>,
    prompt: &str,
    agent: Option<&str>,
    model: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO node_tasks (id, node_id, session_id, title, prompt, agent, model, status, error, cancel_requested, created_at, claimed_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, 0, ?8, NULL, NULL)",
        params![task_id, node_id, session_id, title, prompt, agent, model, now_ms],
    )
    .await
    .context("insert node_task")?;
    Ok(())
}

async fn session_exists(conn: &Connection, session_id: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare_cached("SELECT 1 FROM sessions WHERE id = ?1 LIMIT 1")
        .await?;
    let mut rows = stmt.query(params![session_id]).await?;
    Ok(rows.next().await?.is_some())
}

/// Atomically claim the oldest pending task of `node_id`. Returns `None`
/// when the node already has a running/cancelling task (single-active-task
/// policy) or nothing is queued. The CAS `UPDATE ... WHERE status='pending'`
/// (never RETURNING) makes losing racers see zero rows => `Ok(None)`.
pub async fn claim_next(
    conn: &Connection,
    node_id: &str,
    now_ms: i64,
) -> Result<Option<NodeTaskRecord>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        // (a) Single active task per node.
        {
            let sql = "SELECT 1 FROM node_tasks WHERE node_id = ?1 AND status IN ('running','cancelling') LIMIT 1";
            let mut rows = conn.query(sql, params![node_id]).await?;
            if rows.next().await?.is_some() {
                return Ok(None);
            }
        }
        // (b) Oldest pending task (FIFO). `created_at` has millisecond
        // resolution, so two dispatches in the same ms tie; the ULID is NOT a
        // stable tiebreak (not monotonic across calls), so use the implicit
        // insertion-order `rowid` instead.
        let next_id: Option<String> = {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id FROM node_tasks WHERE node_id = ?1 AND status = 'pending'
                     ORDER BY created_at ASC, rowid ASC LIMIT 1",
                )
                .await?;
            let mut rows = stmt.query(params![node_id]).await?;
            match rows.next().await? {
                Some(r) => Some(r.get::<String>(0)?),
                None => None,
            }
        };
        let Some(task_id) = next_id else {
            return Ok(None);
        };
        // (c) Compare-and-swap pending -> running.
        let changed = conn
            .execute(
                "UPDATE node_tasks SET status = 'running', claimed_at = ?1 WHERE id = ?2 AND status = 'pending'",
                params![now_ms, task_id.as_str()],
            )
            .await
            .context("claim node_task")?;
        if changed == 0 {
            return Ok(None); // lost the race — treat as "nothing claimed"
        }
        // (d) Mark the node busy, then (e) read the claimed row back.
        conn.execute(
            "UPDATE nodes SET last_status = 'busy', last_task_id = ?2 WHERE id = ?1",
            params![node_id, task_id.as_str()],
        )
        .await?;
        get_task(conn, &task_id).await
    })
    .await
}

/// Apply a validated transition atomically. Terminal writes stamp
/// `finished_at` and release a matching busy slot (`last_status='idle'`);
/// `last_task_id` stays put so UIs can keep showing recent work.
pub async fn update_status(
    conn: &Connection,
    task_id: &str,
    to: NodeTaskStatus,
    error: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let from = current_status(conn, task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("update_node_task_status: task {task_id} not found"))?;
        if !transition_allowed(from, to) {
            anyhow::bail!(
                "illegal node task transition {} -> {} for task {task_id}",
                from.as_str(),
                to.as_str()
            );
        }
        conn.execute(
            "UPDATE node_tasks SET status = ?2, error = ?3, finished_at = ?4 WHERE id = ?1",
            params![
                task_id,
                to.as_str(),
                error,
                if to.is_terminal() { Some(now_ms) } else { None }
            ],
        )
        .await
        .context("update node_task status")?;
        conn.execute(
            "UPDATE nodes SET last_status = 'idle'
             WHERE id = (SELECT node_id FROM node_tasks WHERE id = ?1) AND last_task_id = ?1",
            params![task_id],
        )
        .await
        .context("release busy node after terminal transition")?;
        Ok(())
    })
    .await
}

/// Collapse zombie tasks of lost nodes: every `running`/`cancelling` task of
/// a node whose latest heartbeat is older than `stale_ms` becomes
/// `error("node lost")`, stamped `finished_at` and releasing the node's busy
/// slot (same housekeeping as terminal [`update_status`] writes). The age
/// bound is exclusive — exactly `stale_ms` old still counts as fresh —
/// matching the web-side liveness display. `pending` rows are intentionally
/// left alone (they never block claiming); terminal rows are frozen, so the
/// sweep is idempotent.
pub async fn converge_lost(
    conn: &Connection,
    now_ms: i64,
    stale_ms: i64,
) -> Result<Vec<NodeTaskRecord>> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        // (a) Snapshot candidate ids: active tasks of heartbeat-stale nodes.
        let ids = {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT node_tasks.id FROM node_tasks
                     JOIN nodes ON node_tasks.node_id = nodes.id
                     WHERE (?1 - nodes.last_seen_at) > ?2
                       AND node_tasks.status IN ('running','cancelling')",
                )
                .await?;
            let mut rows = stmt.query(params![now_ms, stale_ms]).await?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().await? {
                out.push(r.get::<String>(0)?);
            }
            out
        };
        // (b) Conditional CAS per row (no RETURNING, same as claim), then
        // release the busy slot exactly like terminal writes do.
        for id in &ids {
            let changed = conn
                .execute(
                    "UPDATE node_tasks SET status = 'error', error = 'node lost', finished_at = ?2
                     WHERE id = ?1 AND status IN ('running','cancelling')",
                    params![id.as_str(), now_ms],
                )
                .await
                .context("converge lost node_task")?;
            if changed == 0 {
                continue; // raced away between select and update
            }
            conn.execute(
                "UPDATE nodes SET last_status = 'idle'
                 WHERE id = (SELECT node_id FROM node_tasks WHERE id = ?1) AND last_task_id = ?1",
                params![id.as_str()],
            )
            .await
            .context("release busy node after lost-node convergence")?;
        }
        // (c) Read the converged rows back in selection order.
        let mut out = Vec::new();
        for id in &ids {
            if let Some(record) = get_task(conn, id).await? {
                out.push(record);
            }
        }
        Ok(out)
    })
    .await
}

/// Bulk-clear a node's console dialogs in one transaction: delete the
/// sessions of every TERMINAL task (done | error | cancelled) of `node_id`;
/// sessions of pending/running/cancelling tasks are kept and reported as
/// `skipped`. The `sessions` delete cascades to the node_tasks row and all
/// child tables, so no per-row work is needed.
pub async fn clear_finished_sessions(conn: &Connection, node_id: &str) -> Result<ClearNodeDialogs> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let skipped = {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT session_id FROM node_tasks
                     WHERE node_id = ?1 AND status NOT IN ('done','error','cancelled')
                     ORDER BY created_at DESC",
                )
                .await?;
            let mut rows = stmt.query(params![node_id]).await?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().await? {
                out.push(r.get::<String>(0)?);
            }
            out
        };
        let removed = conn
            .execute(
                "DELETE FROM sessions WHERE id IN (
                   SELECT session_id FROM node_tasks
                   WHERE node_id = ?1 AND status IN ('done','error','cancelled')
                 )",
                params![node_id],
            )
            .await
            .context("clear finished node sessions")?;
        Ok(ClearNodeDialogs {
            removed: removed as u64,
            skipped,
        })
    })
    .await
}
