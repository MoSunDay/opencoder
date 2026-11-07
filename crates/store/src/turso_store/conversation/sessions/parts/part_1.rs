const INSERT_SESSION: &str = "\
INSERT OR IGNORE INTO sessions (id, kind, title, agent, model, autopilot_mode, workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, handoff_plan, skill, task_type, requirement)
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
    let count = conn
        .execute(
            "UPDATE sessions SET harness_runtime = ? WHERE id = ?",
            params![serde_json::to_string(runtime)?, id],
        )
        .await?;
    anyhow::ensure!(count == 1, "session missing while saving harness runtime");
    Ok(())
}

pub async fn create(conn: &Connection, meta: &SessionMeta) -> Result<()> {
    conn.execute(
        INSERT_SESSION,
        params![
            meta.id.as_str(),
            meta.kind.as_deref(),
            meta.title.as_deref(),
            meta.agent.as_deref(),
            meta.model.as_deref(),
            meta.autopilot_mode.as_deref(),
            meta.workdir_hash.as_deref(),
            meta.created_at,
            meta.updated_at,
            meta.summary.as_deref(),
            meta.summary_seq,
            serde_json::to_string(&meta.summary_images).unwrap_or_else(|_| "[]".into()),
            meta.handoff_seq,
            meta.handoff_plan.as_deref(),
            meta.skill.as_deref(),
            meta.task_type.as_deref().unwrap_or("parent"),
            meta.requirement.as_deref(),
        ],
    )
    .await
    .context("insert session")?;
    Ok(())
}

pub async fn get(conn: &Connection, id: &str) -> Result<Option<SessionMeta>> {
    let mut stmt = conn
        .prepare_cached("SELECT id, kind, title, agent, model, workdir_hash, created_at, updated_at, summary, summary_seq, summary_images_json, handoff_seq, handoff_plan, skill, task_type, requirement, autopilot_mode FROM sessions WHERE id = ?")
        .await?;
    let mut rows = stmt.query(params![id]).await?;
    match rows.next().await? {
        Some(r) => Ok(Some(row_to_meta(&r)?)),
        None => Ok(None),
    }
}

pub async fn list(conn: &Connection, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
    list_inner(conn, filter, false).await
}

/// Execution inventory includes DAG Agent-step sessions in addition to
/// top-level sessions. It remains separate from the public chat listing so
/// internal execution details cannot become user-editable conversations.
pub async fn list_execution_sessions(
    conn: &Connection,
    filter: &SessionFilter,
) -> Result<Vec<SessionListItem>> {
    list_inner(conn, filter, true).await
}

async fn list_inner(
    conn: &Connection,
    filter: &SessionFilter,
    include_execution_details: bool,
) -> Result<Vec<SessionListItem>> {
    let limit = filter.limit.clamp(1, 500) as i64;
    let mut where_clauses: Vec<String> = Vec::new();
    let mut args: Vec<Value> = Vec::new();

    if let Some(h) = &filter.workdir_hash {
        where_clauses.push("s.workdir_hash = ?".into());
        args.push(h.clone().into());
    }
    if let Some(s) = &filter.search {
        // Escape LIKE metacharacters (`\`, `%`, `_`) in the user-supplied term
        // so they match literally. `ESCAPE '\'` declares the escape character.
        where_clauses
            .push("(s.id LIKE ? ESCAPE '\\' OR COALESCE(s.title,'') LIKE ? ESCAPE '\\')".into());
        let escaped = s
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like = format!("%{escaped}%");
        args.push(like.clone().into());
        args.push(like.into());
    }
    if let Some(cursor) = &filter.cursor {
        // A malformed cursor is a real error, not a silent fallback to page 1.
        // Cursor ts is the *activity* key (see ACTIVITY_EXPR), matching the
        // ORDER BY so keyset pagination is stable across pages.
        let (ts, id) = decode_cursor(cursor)
            .ok_or_else(|| anyhow::anyhow!("invalid list cursor: {cursor}"))?;
        where_clauses.push(format!(
            "({ACTIVITY_EXPR} < ? OR ({ACTIVITY_EXPR} = ? AND s.id < ?))"
        ));
        args.push(ts.into());
        args.push(ts.into());
        args.push(id.into());
    }
    // `include_subagents` widens the visible types from "top-level parents
    // only" to parents + subagent children. DAG Agent-step sessions are
    // durable execution detail and remain queryable by their session ID; they
    // are filtered from the chat lane by the control API's visibility fence.
    if include_execution_details {
        where_clauses.push(format!(
            "((s.task_type = '{TASK_TYPE_PARENT}' AND NOT EXISTS (SELECT 1 FROM subagent_tasks st WHERE st.child_session_id = s.id)) OR s.task_type = '{TASK_TYPE_AGENT_STEP}')"
        ));
    } else if filter.include_subagents {
        where_clauses.push(format!(
            "s.task_type IN ('{TASK_TYPE_PARENT}','{TASK_TYPE_SUBAGENT}')"
        ));
    } else {
        where_clauses.push(format!(
            "((s.task_type = '{TASK_TYPE_PARENT}' AND NOT EXISTS (SELECT 1 FROM subagent_tasks st WHERE st.child_session_id = s.id)) OR s.task_type = '{TASK_TYPE_AGENT_STEP}')"
        ));
    }

    // Lane fence: operator-tagged sessions are execution detail, never chat
    // history, so every lane hides them unless explicitly requested. Legacy
    // rows (kind NULL) keep the caller-side prefix resolution.
    match filter.kind.as_deref() {
        Some(kind) => {
            where_clauses.push("s.kind = ?".into());
            args.push(kind.into());
        }
        None => where_clauses.push("(s.kind IS NULL OR s.kind <> 'operator')".into()),
    }
    // Walk the activity index until the page is full. The task-type index
    // otherwise makes Turso sort every matching session before LIMIT. Keep
    // previews outside that page so discarded sessions never load messages.
    let mut sql = String::from(
        "WITH page AS MATERIALIZED (SELECT s.id, s.kind, s.title, s.agent, s.model, \
         s.created_at, s.updated_at, s.skill FROM sessions s INDEXED BY idx_sessions_activity",
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    // Short JSON needs no character-by-character substring copy. Preserve
    // substr's character limit and embedded-NUL behavior for all other rows.
    sql.push_str(&format!(
        " ORDER BY {ACTIVITY_EXPR} DESC, s.id DESC LIMIT ?) \
         SELECT s.id, s.kind, s.title, s.agent, s.model, s.created_at, s.updated_at, \
         (SELECT CASE WHEN octet_length(m.blocks_json) <= 8192 AND instr(m.blocks_json, char(0)) = 0 \
          THEN m.blocks_json ELSE substr(m.blocks_json, 1, 8192) END FROM messages m \
          WHERE m.session_id = s.id AND m.role = 'user' ORDER BY m.seq ASC LIMIT 1), s.skill \
         FROM page s ORDER BY {ACTIVITY_EXPR} DESC, s.id DESC"
    ));
    args.push(limit.into());

    let mut stmt = conn.prepare_cached(&sql).await?;
    let mut rows = stmt.query(params_from_iter(args)).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(SessionListItem {
            id: r.get::<String>(0)?,
            kind: r.get::<Option<String>>(1)?,
            title: r.get::<Option<String>>(2)?,
            agent: normalize_agent(r.get::<Option<String>>(3)?),
            model: r.get::<Option<String>>(4)?,
            created_at: r.get::<i64>(5)?,
            updated_at: r.get::<i64>(6)?,
            preview: extract_preview(&r.get::<Option<String>>(7)?),
            skill: r.get::<Option<String>>(8)?,
        });
    }
    Ok(out)
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
    if patch.autopilot_mode.is_some() && patch.clear_autopilot_mode {
        anyhow::bail!(
            "SessionPatch: autopilot_mode field and clear_autopilot_mode are mutually exclusive"
        );
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
    if let Some(v) = &patch.autopilot_mode {
        sets.push("autopilot_mode = ?");
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
    if patch.clear_autopilot_mode {
        sets.push("autopilot_mode = NULL");
    }
    if let Some(v) = patch.updated_at {
        sets.push("updated_at = ?");
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
    conn.execute(&sql, params_from_iter(args)).await?;
    Ok(())
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
    let affected = conn
        .execute("DELETE FROM sessions WHERE id != ?", params![keep_id])
        .await
        .context("clear other sessions")?;
    Ok(affected as u64)
}
