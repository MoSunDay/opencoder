// ---- todo runs ----

pub async fn create_todo_run(conn: &Connection, rec: &ProjectTodoRunRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO project_todo_runs (id, todo_id, kind, version, plan_md, output_md, agent, session_id, status, started_at, finished_at, created_at, executor_kind, capability_id, plan_id, output_ref, input_snapshot, trace_manifest) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            rec.id.as_str(),
            rec.todo_id.as_str(),
            rec.kind.as_str(),
            rec.version,
            rec.plan_md.as_deref(),
            rec.output_md.as_deref(),
            rec.agent.as_str(),
            rec.session_id.as_deref(),
            rec.status.as_str(),
            rec.started_at,
            rec.finished_at,
            rec.created_at,
            rec.executor_kind.as_str(),
            rec.capability_id.as_deref(),
            rec.plan_id.as_deref(),
            rec.output_ref.as_deref(),
            rec.input_snapshot.as_deref(),
            rec.trace_manifest.as_deref()
        ],
    )
    .await
    .context("insert project todo run")?;
    Ok(())
}

/// The `SET` clause fragments + bound values shared by `patch_todo_run` and
/// its expected-status CAS variant — pure projection of the patch's `Some`
/// fields, no I/O. Plain `Option<String>` fields set, never clear.
fn run_set_fragment(patch: &ProjectTodoRunPatch) -> (Vec<&'static str>, Vec<Value>) {
    let mut sets: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Value> = Vec::new();
    if let Some(v) = patch.input_snapshot.as_deref() {
        sets.push("input_snapshot = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.trace_manifest.as_deref() {
        sets.push("trace_manifest = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.plan_md.as_deref() {
        sets.push("plan_md = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.output_md.as_deref() {
        sets.push("output_md = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.output_ref.as_deref() {
        sets.push("output_ref = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.capability_id.as_deref() {
        sets.push("capability_id = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.plan_id.as_deref() {
        sets.push("plan_id = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.session_id.as_deref() {
        sets.push("session_id = ?");
        vals.push(v.into());
    }
    if let Some(v) = patch.status {
        sets.push("status = ?");
        vals.push(v.as_str().into());
    }
    if let Some(v) = patch.finished_at {
        sets.push("finished_at = ?");
        vals.push(v.into());
    }
    (sets, vals)
}

pub async fn finish_todo_run(
    conn: &Connection,
    id: &str,
    patch: &ProjectTodoRunPatch,
    now_ms: i64,
) -> Result<bool> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let run = get_todo_run(conn, id)
            .await?
            .context("run missing during finalization")?;
        if run.status != ProjectTodoRunStatus::Running {
            return Ok(false);
        }
        let status = patch.status.context("final run status missing")?;
        anyhow::ensure!(
            status != ProjectTodoRunStatus::Running,
            "run finalization requires terminal status"
        );
        // Step runs (playbook 子尝试) 永不回写 todo 状态：todo 生命周期由
        // 父 playbook run 独占（它才是 Execute 行）；Plan/Execute 语义不变。
        let next = if run.kind == ProjectTodoRunKind::Plan {
            (status == ProjectTodoRunStatus::Done).then_some(ProjectTodoStatus::Planned)
        } else if run.kind == ProjectTodoRunKind::Step {
            None
        } else {
            Some(match status {
                ProjectTodoRunStatus::Done => ProjectTodoStatus::Done,
                ProjectTodoRunStatus::Cancelled => ProjectTodoStatus::Planned,
                _ => ProjectTodoStatus::Failed,
            })
        };
        if let Some(status) = next {
            let current = get_todo(conn, &run.todo_id)
                .await?
                .context("todo missing during finalization")?;
            let todo_patch = crate::ProjectTodoPatch {
                people: Default::default(),
                status: Some(status),
                plan_md: if run.kind == ProjectTodoRunKind::Plan {
                    Some(patch.output_md.clone())
                } else {
                    None
                },
                ..Default::default()
            };
            if run.kind == ProjectTodoRunKind::Plan || current.status == ProjectTodoStatus::Running
            {
                anyhow::ensure!(
                    patch_todo(conn, &run.todo_id, &todo_patch, now_ms).await?,
                    "todo missing during finalization"
                );
            }
        }
        patch_todo_run(conn, id, patch, now_ms).await
    })
    .await
}

pub async fn patch_todo_run(
    conn: &Connection,
    id: &str,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let (sets, mut vals) = run_set_fragment(patch);
    // The runs table has no updated_at column (created_at + finished_at span
    // its lifecycle), so the now_ms parameter stays unused; it exists for
    // signature uniformity with the other patch_* methods.
    let sql = format!(
        "UPDATE project_todo_runs SET {} WHERE id = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project todo run")?;
    Ok(n > 0)
}

/// Expected-status CAS variant of `patch_todo_run`: `WHERE id = ? AND
/// status = ?`. Applies only while the run row still holds `when` — a
/// stale convergence must not relabel a row the driver already closed.
pub async fn patch_todo_run_when(
    conn: &Connection,
    id: &str,
    when: ProjectTodoRunStatus,
    patch: &ProjectTodoRunPatch,
    _now_ms: i64,
) -> Result<bool> {
    let (sets, mut vals) = run_set_fragment(patch);
    // Runs have no updated_at; `_now_ms` stays unused for signature
    // uniformity with the other patch_* methods.
    let sql = format!(
        "UPDATE project_todo_runs SET {} WHERE id = ? AND status = ?",
        sets.join(", ")
    );
    vals.push(id.into());
    vals.push(when.as_str().into());
    let n = conn
        .execute(&sql, vals)
        .await
        .context("patch project todo run (expected status)")?;
    Ok(n > 0)
}

pub async fn get_todo_run(conn: &Connection, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE id = ?1 LIMIT 1"
        ))
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_run(&r)?)),
        None => Ok(None),
    }
}

pub async fn get_todo_run_summary(
    conn: &Connection,
    id: &str,
) -> Result<Option<crate::ProjectTodoRunSummary>> {
    let mut rows = conn
        .query(
            "SELECT id,todo_id,kind,version, \
             CASE WHEN length(CAST(plan_md AS BLOB))<=65536 THEN plan_md END, \
             length(CAST(plan_md AS BLOB)), \
             CASE WHEN length(CAST(output_md AS BLOB))<=65536 THEN output_md END, \
             length(CAST(output_md AS BLOB)),agent,session_id,status,started_at,finished_at,created_at, \
             executor_kind,capability_id,plan_id,output_ref, \
             CASE WHEN length(CAST(input_snapshot AS BLOB))<=65536 THEN input_snapshot END, length(CAST(input_snapshot AS BLOB)), \
             CASE WHEN length(CAST(trace_manifest AS BLOB))<=65536 THEN trace_manifest END, length(CAST(trace_manifest AS BLOB)) \
             FROM project_todo_runs WHERE id=?1 LIMIT 1",
            params![id],
        )
        .await?;
    rows.next()
        .await?
        .map(|row| row_to_run_summary(&row))
        .transpose()
}

/// Newest version first.
pub async fn list_todo_runs(conn: &Connection, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE todo_id = ?1 ORDER BY version DESC"
        ))
        .await?;
    let mut rows = stmt.query(params![todo_id]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_run(&r)?);
    }
    Ok(out)
}

pub async fn list_todo_runs_page(
    conn: &Connection,
    todo_id: &str,
    before_version: Option<i64>,
    limit: u32,
) -> Result<crate::ProjectTodoRunPage> {
    let limit = limit.clamp(1, 100) as usize;
    let mut rows = conn
        .query(
            "SELECT id,todo_id,kind,version, \
             CASE WHEN length(CAST(plan_md AS BLOB))<=65536 THEN plan_md END, \
             length(CAST(plan_md AS BLOB)), \
             CASE WHEN length(CAST(output_md AS BLOB))<=65536 THEN output_md END, \
             length(CAST(output_md AS BLOB)),agent,session_id,status,started_at,finished_at,created_at, \
             executor_kind,capability_id,plan_id,output_ref, \
             CASE WHEN length(CAST(input_snapshot AS BLOB))<=65536 THEN input_snapshot END, length(CAST(input_snapshot AS BLOB)), \
             CASE WHEN length(CAST(trace_manifest AS BLOB))<=65536 THEN trace_manifest END, length(CAST(trace_manifest AS BLOB)) \
             FROM project_todo_runs WHERE todo_id=?1 AND (?2 IS NULL OR version<?2) \
             ORDER BY version DESC LIMIT ?3",
            params![todo_id, before_version, limit as i64 + 1],
        )
        .await?;
    let mut out = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().await? {
        out.push(row_to_run_summary(&row)?);
    }
    crate::project_types::project_run_page(out, limit)
}

pub async fn project_text_chunk(
    conn: &Connection,
    record_kind: &str,
    owner_id: &str,
    id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let (table, column) = match (record_kind, field) {
        ("todo", "draft") => ("project_todos", "draft"),
        ("todo", "plan_md") => ("project_todos", "plan_md"),
        ("run", "plan_md") => ("project_todo_runs", "plan_md"),
        ("run", "output_md") => ("project_todo_runs", "output_md"),
        ("run", "input_snapshot") => ("project_todo_runs", "input_snapshot"),
        ("run", "trace_manifest") => ("project_todo_runs", "trace_manifest"),
        _ => anyhow::bail!("unsupported project text field"),
    };
    let start = i64::try_from(offset)?.saturating_add(1);
    let take = max_bytes.clamp(1, 64 * 1024) as i64;
    let owner_clause = if record_kind == "run" {
        " AND todo_id=?4"
    } else {
        " AND id=?4"
    };
    let mut rows = conn
        .query(
            &format!(
                "SELECT length(CAST({column} AS BLOB)), \
                 CAST(substr(CAST({column} AS BLOB),?2,?3) AS BLOB) FROM {table} \
                 WHERE id=?1{owner_clause}"
            ),
            params![id, start, take, owner_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let Some(total) = row.get::<Option<i64>>(0)? else {
        return Ok(None);
    };
    let total = total.max(0) as u64;
    if offset > total {
        anyhow::bail!("project text offset exceeds total bytes");
    }
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: row.get::<Option<Vec<u8>>>(1)?.unwrap_or_default(),
    }))
}

/// Every run row currently in the `running` state (any todo, any kind) —
/// feeds the opportunistic stale-run sweep.
pub async fn list_running_todo_runs(conn: &Connection) -> Result<Vec<ProjectTodoRunRecord>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT {RUN_COLS} FROM project_todo_runs WHERE status = ?1"
        ))
        .await?;
    let mut rows = stmt
        .query(params![ProjectTodoRunStatus::Running.as_str()])
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(row_to_run(&r)?);
    }
    Ok(out)
}
