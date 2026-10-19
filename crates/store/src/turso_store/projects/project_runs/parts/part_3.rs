/// `COALESCE(MAX(version), 0) + 1` — 1 for a todo with no runs yet.
pub async fn next_todo_version(conn: &Connection, todo_id: &str) -> Result<i64> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM project_todo_runs WHERE todo_id = ?1",
        )
        .await
        .context("prepare next_todo_version")?;
    let mut rows = stmt
        .query(params![todo_id])
        .await
        .context("query next_todo_version")?;
    match rows.next().await? {
        Some(r) => Ok(r.get(0)?),
        None => Ok(1), // unreachable: the aggregate always yields a row
    }
}

fn row_to_run(r: &turso::Row) -> Result<ProjectTodoRunRecord> {
    Ok(ProjectTodoRunRecord {
        input_snapshot: r.get(16)?,
        trace_manifest: r.get(17)?,
        id: r.get(0)?,
        todo_id: r.get(1)?,
        kind: ProjectTodoRunKind::parse(&r.get::<String>(2)?).context("project_todo_runs.kind")?,
        version: r.get(3)?,
        plan_md: r.get(4)?,
        output_md: r.get(5)?,
        agent: r.get(6)?,
        executor_kind: ProjectExecutorKind::parse(&r.get::<String>(12)?)
            .context("project_todo_runs.executor_kind")?,
        capability_id: r.get(13)?,
        plan_id: r.get(14)?,
        output_ref: r.get(15)?,
        session_id: r.get(7)?,
        status: ProjectTodoRunStatus::parse(&r.get::<String>(8)?)
            .context("project_todo_runs.status")?,
        started_at: r.get(9)?,
        finished_at: r.get(10)?,
        created_at: r.get(11)?,
    })
}

fn row_to_run_summary(r: &turso::Row) -> Result<crate::ProjectTodoRunSummary> {
    let id: String = r.get(0)?;
    Ok(crate::ProjectTodoRunSummary {
        input_snapshot: summary_text(r.get(18)?, r.get(19)?, &id, "input_snapshot"),
        trace_manifest: summary_text(r.get(20)?, r.get(21)?, &id, "trace_manifest"),
        id: id.clone(),
        todo_id: r.get(1)?,
        kind: ProjectTodoRunKind::parse(&r.get::<String>(2)?).context("project_todo_runs.kind")?,
        version: r.get(3)?,
        plan_md: summary_text(r.get(4)?, r.get(5)?, &id, "plan_md"),
        output_md: summary_text(r.get(6)?, r.get(7)?, &id, "output_md"),
        agent: r.get(8)?,
        executor_kind: ProjectExecutorKind::parse(&r.get::<String>(14)?)
            .context("project_todo_runs.executor_kind")?,
        capability_id: r.get(15)?,
        plan_id: r.get(16)?,
        output_ref: r.get(17)?,
        session_id: r.get(9)?,
        status: ProjectTodoRunStatus::parse(&r.get::<String>(10)?)
            .context("project_todo_runs.status")?,
        started_at: r.get(11)?,
        finished_at: r.get(12)?,
        created_at: r.get(13)?,
    })
}
