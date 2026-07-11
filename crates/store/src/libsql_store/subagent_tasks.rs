use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::{SubagentStatus, SubagentTaskRecord};

const INSERT: &str = "\
INSERT INTO subagent_tasks \
  (task_id, parent_session_id, child_session_id, parent_message_id, agent, prompt, result, status, ok, started_at, completed_at) \
VALUES (?, ?, ?, ?, ?, ?, NULL, ?, NULL, ?, NULL)";

const COMPLETE: &str = "\
UPDATE subagent_tasks SET result = ?1, ok = ?2, status = ?3, completed_at = ?4 WHERE task_id = ?5";

const SELECT_BY_PARENT: &str = "\
SELECT task_id, parent_session_id, child_session_id, parent_message_id, agent, prompt, result, status, ok, started_at, completed_at \
FROM subagent_tasks WHERE parent_session_id = ?1 ORDER BY seq ASC";

pub async fn create(conn: &Connection, rec: &SubagentTaskRecord) -> Result<()> {
    let parent_msg: Option<&str> = rec.parent_message_id.as_deref();
    conn.execute(
        INSERT,
        params![
            rec.task_id.as_str(),
            rec.parent_session_id.as_str(),
            rec.child_session_id.as_str(),
            parent_msg,
            rec.agent.as_str(),
            rec.prompt.as_str(),
            rec.status.as_str(),
            rec.started_at,
        ],
    )
    .await
    .context("insert subagent_task")?;
    Ok(())
}

pub async fn complete(conn: &Connection, task_id: &str, result: &str, ok: bool) -> Result<()> {
    let status = if ok {
        SubagentStatus::Completed
    } else {
        SubagentStatus::Failed
    };
    let now = opencode_core::message::now_ms();
    conn.execute(COMPLETE, params![result, ok, status.as_str(), now, task_id])
        .await
        .context("update subagent_task completion")?;
    Ok(())
}

pub async fn list(conn: &Connection, parent_session_id: &str) -> Result<Vec<SubagentTaskRecord>> {
    let stmt = conn
        .prepare(SELECT_BY_PARENT)
        .await
        .context("prepare subagent_tasks select")?;
    let mut rows = stmt
        .query(params![parent_session_id])
        .await
        .context("query subagent_tasks")?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let status_str: String = row.get(7)?;
        out.push(SubagentTaskRecord {
            task_id: row.get(0)?,
            parent_session_id: row.get(1)?,
            child_session_id: row.get(2)?,
            parent_message_id: row.get(3)?,
            agent: row.get(4)?,
            prompt: row.get(5)?,
            result: row.get(6)?,
            status: SubagentStatus::parse(&status_str),
            ok: row.get::<Option<i64>>(8)?.map(|v| v != 0),
            started_at: row.get(9)?,
            completed_at: row.get(10)?,
        });
    }
    Ok(out)
}
