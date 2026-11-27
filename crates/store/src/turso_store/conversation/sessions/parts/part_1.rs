const INSERT_SESSION: &str = "\
INSERT OR IGNORE INTO sessions (id, kind, title, agent, model, workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, handoff_plan, skill, task_type, requirement, model_overrides_json)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

pub async fn harness_runtime(
    conn: &Connection,
    id: &str,
) -> Result<Option<opencoder_core::harness::HarnessRuntime>> {
    let mut rows = conn
        .query(
            "SELECT harness_runtime FROM sessions WHERE id = ?",
            params![id],
        )
        .await?;
    let raw = match rows.next().await? {
        Some(row) => row.get::<Option<String>>(0)?,
        None => None,
    };
    raw.map(|s| serde_json::from_str(&s).context("invalid stored harness runtime"))
        .transpose()
}

pub async fn set_harness_runtime(
    conn: &Connection,
    id: &str,
    runtime: &opencoder_core::harness::HarnessRuntime,
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let count = conn
            .execute(
                "UPDATE sessions SET harness_runtime = ? WHERE id = ?",
                params![serde_json::to_string(runtime)?, id],
            )
            .await?;
        anyhow::ensure!(count == 1, "session missing while saving harness runtime");
        Ok(())
    })
    .await
}

/// Insert the session row inside a caller-owned transaction.
pub(super) async fn create_in_tx(conn: &Connection, meta: &SessionMeta) -> Result<()> {
    create_in_tx_with_model_overrides(conn, meta, None).await
}

pub(super) async fn create_in_tx_with_model_overrides(
    conn: &Connection,
    meta: &SessionMeta,
    overrides: Option<&crate::SessionModelOverrides>,
) -> Result<()> {
    let overrides_json = overrides
        .map(serde_json::to_string)
        .transpose()
        .context("serialize session model overrides")?;
    conn.execute(
        INSERT_SESSION,
        params![
            meta.id.as_str(),
            meta.kind.as_deref(),
            meta.title.as_deref(),
            meta.agent.as_deref(),
            meta.model.as_deref(),
            meta.workdir_hash.as_deref(),
            meta.created_at,
            // `updated_at` is the activity sort key (see ACTIVITY_EXPR); keep
            // it >= `created_at` so backfilled rows (updated_at = 0) order by
            // their creation time exactly like the old MAX() expression did.
            meta.updated_at.max(meta.created_at),
            meta.summary.as_deref(),
            meta.summary_seq,
            serde_json::to_string(&meta.summary_images).unwrap_or_else(|_| "[]".into()),
            meta.handoff_seq,
            meta.handoff_plan.as_deref(),
            meta.skill.as_deref(),
            meta.task_type.as_deref().unwrap_or("parent"),
            meta.requirement.as_deref(),
            overrides_json.as_deref(),
        ],
    )
    .await
    .context("insert session")?;
    Ok(())
}

pub async fn create(conn: &Connection, meta: &SessionMeta) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || create_in_tx(conn, meta)).await
}

pub async fn create_with_model_overrides(
    conn: &Connection,
    meta: &SessionMeta,
    overrides: Option<&crate::SessionModelOverrides>,
) -> Result<()> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || {
        create_in_tx_with_model_overrides(conn, meta, overrides)
    })
    .await
}

pub async fn get_model_overrides(
    conn: &Connection,
    id: &str,
) -> Result<Option<crate::SessionModelOverrides>> {
    let mut rows = conn
        .query(
            "SELECT model_overrides_json FROM sessions WHERE id = ?",
            params![id],
        )
        .await?;
    let raw = match rows.next().await? {
        Some(row) => row.get::<Option<String>>(0)?,
        None => None,
    };
    raw.map(|json| serde_json::from_str(&json).context("invalid stored session model overrides"))
        .transpose()
}

pub async fn set_model_overrides(
    conn: &Connection,
    id: &str,
    overrides: Option<&crate::SessionModelOverrides>,
) -> Result<()> {
    let json = overrides
        .map(serde_json::to_string)
        .transpose()
        .context("serialize session model overrides")?;
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let count = conn
            .execute(
                "UPDATE sessions SET model_overrides_json = ? WHERE id = ?",
                params![json.as_deref(), id],
            )
            .await?;
        anyhow::ensure!(count == 1, "session missing while saving model overrides");
        Ok(())
    })
    .await
}

pub async fn get(conn: &Connection, id: &str) -> Result<Option<SessionMeta>> {
    let mut stmt = conn
        .prepare_cached("SELECT id, kind, title, agent, model, workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, handoff_plan, skill, task_type, requirement FROM sessions WHERE id = ?")
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_meta(&r)?)),
        None => Ok(None),
    }
}

pub async fn list(conn: &Connection, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
    listing::list(conn, filter, false).await
}

/// Execution inventory includes DAG Agent-step sessions in addition to
/// top-level sessions. It remains separate from the public chat listing so
/// internal execution details cannot become user-editable conversations.
pub async fn list_execution_sessions(
    conn: &Connection,
    filter: &SessionFilter,
) -> Result<Vec<SessionListItem>> {
    listing::list(conn, filter, true).await
}

pub async fn update(conn: &Connection, id: &str, patch: &SessionPatch) -> Result<()> {
    // Validate: a field value and the clear flag for the same column are
    // contradictory — they would emit both `col = ?` and `col = NULL` SET
    // clauses, producing order-dependent SQL. Reject these combos up front.
    if patch.summary.is_some() && patch.clear_summary {
        anyhow::bail!("SessionPatch: summary field and clear_summary are mutually exclusive");
    }
    if patch.summary_seq.is_some() && patch.clear_summary {
        anyhow::bail!("SessionPatch: summary_seq field and clear_summary are mutually exclusive");
    }
    if patch.summary_images.is_some() && patch.clear_summary {
        anyhow::bail!(
            "SessionPatch: summary_images field and clear_summary are mutually exclusive"
        );
    }
    if patch.handoff_plan.is_some() && patch.clear_handoff {
        anyhow::bail!("SessionPatch: handoff_plan field and clear_handoff are mutually exclusive");
    }
    if patch.handoff_seq.is_some() && patch.clear_handoff {
        anyhow::bail!("SessionPatch: handoff_seq field and clear_handoff are mutually exclusive");
    }
    if patch.skill.is_some() && patch.clear_skill {
        anyhow::bail!("SessionPatch: skill field and clear_skill are mutually exclusive");
    }
    if patch.agent.is_some() && patch.clear_agent {
        anyhow::bail!("SessionPatch: agent field and clear_agent are mutually exclusive");
    }
    if patch.model.is_some() && patch.clear_model {
        anyhow::bail!("SessionPatch: model field and clear_model are mutually exclusive");
    }
    if patch.requirement.is_some() && patch.clear_requirement {
        anyhow::bail!(
            "SessionPatch: requirement field and clear_requirement are mutually exclusive"
        );
    }

    let mut sets: Vec<&str> = Vec::new();
    let mut args: Vec<Value> = Vec::new();
    if let Some(v) = &patch.title {
        sets.push("title = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.agent {
        sets.push("agent = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.model {
        sets.push("model = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = &patch.summary {
        sets.push("summary = ?");
        args.push(v.clone().into());
    }
    if let Some(v) = patch.summary_seq {
        sets.push("summary_seq = ?");
        args.push(v.into());
    }
    if let Some(v) = &patch.summary_images {
        sets.push("summary_images_json = ?");
        args.push(
            serde_json::to_string(v)
                .unwrap_or_else(|_| "[]".into())
                .into(),
        );
    }
    if let Some(v) = patch.handoff_seq {
        sets.push("handoff_seq = ?");
        args.push(v.into());
    }
    if let Some(v) = &patch.handoff_plan {
        sets.push("handoff_plan = ?");
        args.push(v.clone().into());
    }
    if patch.clear_summary {
        sets.push("summary = NULL");
        sets.push("summary_seq = NULL");
        sets.push("summary_images_json = NULL");
    }
    if patch.clear_handoff {
        sets.push("handoff_seq = NULL");
        sets.push("handoff_plan = NULL");
    }
    if let Some(v) = &patch.skill {
        sets.push("skill = ?");
        args.push(v.clone().into());
    }
    if patch.clear_skill {
        sets.push("skill = NULL");
    }
    if patch.clear_agent {
        sets.push("agent = NULL");
    }
    if patch.clear_model {
        sets.push("model = NULL");
    }
    if let Some(v) = patch.updated_at {
        // Never move the activity stamp below creation time (invariant the
        // plain activity index relies on).
        sets.push("updated_at = MAX(?, created_at)");
        args.push(v.into());
    }
    if let Some(v) = &patch.requirement {
        sets.push("requirement = ?");
        args.push(v.clone().into());
    }
    if patch.clear_requirement {
        sets.push("requirement = NULL");
    }
    if sets.is_empty() {
        return Ok(());
    }
    let sql = format!("UPDATE sessions SET {} WHERE id = ?", sets.join(", "));
    args.push(id.to_string().into());
    // `updated_at` is indexed; if that index desynced (Turso rollback bug),
    // rebuild it and retry instead of failing the patch forever.
    activity::repair_and_retry(conn, || {
        let sql = sql.clone();
        let args = args.clone();
        super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
            conn.execute(&sql, params_from_iter(args)).await?;
            Ok(())
        })
    })
    .await
}

pub async fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE id = ?", params![id])
        .await?;
    Ok(())
}

/// Delete all sessions except `keep_id`. Child rows (messages, inputs, events,
/// subagent_tasks) are removed by their `ON DELETE CASCADE` foreign keys.
/// Returns the count of deleted session rows.
pub async fn clear_others(conn: &Connection, keep_id: &str) -> Result<u64> {
    super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let affected = conn
            .execute("DELETE FROM sessions WHERE id != ?", params![keep_id])
            .await?;
        Ok(affected as u64)
    })
    .await
    .context("clear other sessions")
}
