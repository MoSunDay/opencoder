use anyhow::{Context, Result};
use libsql::{params, Connection};

use crate::types::{SubagentStatus, SubagentTaskRecord};

const INSERT: &str = "\
INSERT INTO subagent_tasks \
  (task_id, parent_session_id, child_session_id, parent_message_id, agent, prompt, result, status, ok, started_at, completed_at) \
VALUES (?, ?, ?, ?, ?, ?, NULL, ?, NULL, ?, NULL)";

const COMPLETE: &str = "\
UPDATE subagent_tasks SET result = ?1, ok = ?2, status = ?3, completed_at = ?4 WHERE task_id = ?5 AND status IN ('running', 'cancelled')";

const SELECT_BY_PARENT: &str = "\
SELECT task_id, parent_session_id, child_session_id, parent_message_id, agent, prompt, result, status, ok, started_at, completed_at \
FROM subagent_tasks WHERE parent_session_id = ?1 ORDER BY seq ASC";

const SELECT_BY_TASK_ID: &str = "\
SELECT task_id, parent_session_id, child_session_id, parent_message_id, agent, prompt, result, status, ok, started_at, completed_at \
FROM subagent_tasks WHERE task_id = ?1 LIMIT 1";

const CANCEL: &str = "UPDATE subagent_tasks SET status = ?1, completed_at = ?2 WHERE task_id = ?3";

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
    let now = opencoder_core::message::now_ms();
    let rows = conn
        .execute(COMPLETE, params![result, ok, status.as_str(), now, task_id])
        .await
        .context("update subagent_task completion")?;
    if rows == 0 {
        anyhow::bail!("subagent_task not found: {task_id}");
    }
    Ok(())
}

pub async fn cancel(conn: &Connection, task_id: &str) -> Result<()> {
    let now = opencoder_core::message::now_ms();
    let rows = conn
        .execute(
            CANCEL,
            params![SubagentStatus::Cancelled.as_str(), now, task_id],
        )
        .await
        .context("cancel subagent_task")?;
    if rows == 0 {
        anyhow::bail!("subagent_task not found: {task_id}");
    }
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

pub async fn get_by_task_id(
    conn: &Connection,
    task_id: &str,
) -> Result<Option<SubagentTaskRecord>> {
    let stmt = conn
        .prepare(SELECT_BY_TASK_ID)
        .await
        .context("prepare subagent_tasks select by task_id")?;
    let mut rows = stmt
        .query(params![task_id])
        .await
        .context("query subagent_tasks by task_id")?;
    if let Some(row) = rows.next().await? {
        let status_str: String = row.get(7)?;
        Ok(Some(SubagentTaskRecord {
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
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{cancel, complete, create, get_by_task_id};
    use crate::libsql_store::LibsqlStore;
    use crate::store::Store;
    use crate::types::{SessionMeta, SubagentStatus, SubagentTaskRecord};

    fn session(id: &str) -> SessionMeta {
        SessionMeta {
            id: id.into(),
            title: Some(id.into()),
            agent: Some("build".into()),
            model: Some("m".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        }
    }

    fn task(task_id: &str) -> SubagentTaskRecord {
        SubagentTaskRecord {
            task_id: task_id.into(),
            parent_session_id: "p1".into(),
            child_session_id: "c1".into(),
            parent_message_id: None,
            agent: "build".into(),
            prompt: "delegate".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn cancel_can_override_terminal_for_timeout_recovery() {
        // The timeout handler in execute.rs intentionally calls cancel AFTER
        // the child's cleanup may have called complete (Completed or Failed).
        // This override (terminal -> Cancelled) is legitimate and must succeed:
        // CANCEL has no status guard so the timeout recovery path works.
        let store = LibsqlStore::open_memory().await.unwrap();
        store.create_session(&session("p1")).await.unwrap();
        store.create_session(&session("c1")).await.unwrap();
        let conn = store.conn().await.unwrap();

        create(&conn, &task("t1")).await.unwrap();
        complete(&conn, "t1", "child-finished-ok", true)
            .await
            .unwrap();
        // Timeout override: Completed -> Cancelled (must succeed).
        cancel(&conn, "t1").await.unwrap();
        let rec = get_by_task_id(&conn, "t1")
            .await
            .unwrap()
            .expect("task must exist");
        assert_eq!(
            rec.status,
            SubagentStatus::Cancelled,
            "timeout override must set Cancelled even from Completed"
        );
    }

    #[tokio::test]
    async fn complete_allows_cancelled_to_completed_resume_path() {
        // resume_and_replay replays a Cancelled task and calls complete to
        // transition it Cancelled -> Completed. The COMPLETE guard allows
        // transitions from both 'running' and 'cancelled' (but not from
        // terminal states like 'completed' or 'failed').
        let store = LibsqlStore::open_memory().await.unwrap();
        store.create_session(&session("p1")).await.unwrap();
        store.create_session(&session("c1")).await.unwrap();
        let conn = store.conn().await.unwrap();

        create(&conn, &task("t2")).await.unwrap();
        // Running -> Cancelled (interrupt)
        cancel(&conn, "t2").await.unwrap();
        // Cancelled -> Completed (resume_and_replay path)
        complete(&conn, "t2", "replayed-result", true)
            .await
            .unwrap();

        let rec = get_by_task_id(&conn, "t2")
            .await
            .unwrap()
            .expect("task must exist");
        assert_eq!(
            rec.status,
            SubagentStatus::Completed,
            "Cancelled -> Completed via resume must succeed"
        );
        assert_eq!(rec.result.as_deref(), Some("replayed-result"));
    }

    #[tokio::test]
    async fn complete_does_not_overwrite_completed_terminal_state() {
        // A late complete must not clobber an already-completed task.
        let store = LibsqlStore::open_memory().await.unwrap();
        store.create_session(&session("p1")).await.unwrap();
        store.create_session(&session("c1")).await.unwrap();
        let conn = store.conn().await.unwrap();

        create(&conn, &task("t3")).await.unwrap();
        complete(&conn, "t3", "first-result", true).await.unwrap();
        // Late complete with different result: rejected by the status guard
        // (0 rows affected) and surfaced as an error, not a silent no-op.
        let late = complete(&conn, "t3", "late-result", false).await;
        assert!(late.is_err(), "late complete on terminal task must error");

        let rec = get_by_task_id(&conn, "t3")
            .await
            .unwrap()
            .expect("task must exist");
        assert_eq!(
            rec.status,
            SubagentStatus::Completed,
            "first completion must survive a late complete"
        );
        assert_eq!(rec.result.as_deref(), Some("first-result"));
        assert!(rec.ok.unwrap_or(false));
    }
}
