//! Incremental upgrades for supported shared storage.
use super::*;

pub(super) async fn migrate(conn: &Connection, from: i64) -> Result<()> {
    if from < 24 {
        add_column_if_absent(conn, "messages", "provider_state_json", "TEXT").await?;
    }
    if from < 22 {
        add_column_if_absent(conn, "sessions", "harness_runtime", "TEXT").await?;
    }
    if from < 21 {
        add_column_if_absent(conn, "project_todo_runs", "input_snapshot", "TEXT").await?;
        add_column_if_absent(conn, "project_todo_runs", "trace_manifest", "TEXT").await?;
    }
    if from < 20 {
        // v20: project todo executor dimension — todos carry which executor
        // (agent/team/dag/brain) drives them plus optional inline spec; runs
        // carry the executor kind, brain provenance and artifact/topic refs.
        add_column_if_absent(
            conn,
            "project_todos",
            "executor_kind",
            "TEXT NOT NULL DEFAULT 'agent'",
        )
        .await?;
        add_column_if_absent(conn, "project_todos", "executor_ref", "TEXT").await?;
        add_column_if_absent(conn, "project_todos", "executor_spec", "TEXT").await?;
        add_column_if_absent(
            conn,
            "project_todo_runs",
            "executor_kind",
            "TEXT NOT NULL DEFAULT 'agent'",
        )
        .await?;
        add_column_if_absent(conn, "project_todo_runs", "capability_id", "TEXT").await?;
        add_column_if_absent(conn, "project_todo_runs", "plan_id", "TEXT").await?;
        add_column_if_absent(conn, "project_todo_runs", "output_ref", "TEXT").await?;
    }
    if from < 19 {
        // v19: the caller-provided input id is an idempotency key scoped to a
        // session. This intentionally fails the whole bootstrap transaction
        // for legacy duplicates, preserving the old rows and schema version.
        conn.execute(CREATE_INDEX_IN_ID, ()).await?;
    }
    if from < 17 {
        // v17: team topic-run ledger (opencoder-team fan-out). CREATE IF NOT
        // EXISTS keeps this idempotent; the index lands in the post-batch.
        conn.execute(CREATE_TEAM_TOPIC_RUNS, ()).await?;
    }
    if from < 16 {
        // v16: DAG workflow tables (defs/runs/events). CREATE IF NOT EXISTS
        // keeps this idempotent; indexes land in the post-batch.
        conn.execute(CREATE_DAG_DEFS, ()).await?;
        conn.execute(CREATE_DAG_RUNS, ()).await?;
        conn.execute(CREATE_DAG_EVENTS, ()).await?;
    }
    if from < 15 {
        // v15: project-module tables (goals/milestones/todos/runs) plus the
        // brain capability library (capabilities/exemplar-inputs/vectors).
        // CREATE IF NOT EXISTS keeps this idempotent; indexes land in the
        // post-batch.
        conn.execute(CREATE_PROJECT_GOALS, ()).await?;
        conn.execute(CREATE_PROJECT_MILESTONES, ()).await?;
        conn.execute(CREATE_PROJECT_TODOS, ()).await?;
        conn.execute(CREATE_PROJECT_TODO_RUNS, ()).await?;
        conn.execute(CREATE_BRAIN_CAPABILITIES, ()).await?;
        conn.execute(CREATE_BRAIN_ENG_INPUTS, ()).await?;
        conn.execute(CREATE_BRAIN_VECTORS, ()).await?;
    }
    if from < 14 {
        // v14: verbatim display text on messages — the echo-side single
        // source of truth. User messages record the raw input (`$skill`
        // tokens included) here while `blocks_json` keeps the post-
        // resolution clean text the LLM consumes. Nullable so existing rows
        // stay valid: display layers fall back to the text blocks.
        add_column_if_absent(conn, "messages", "display", "TEXT").await?;
    }
    if from < 13 {
        // v13: last observed/declared address per node (fleet UI column).
        add_column_if_absent(conn, "nodes", "last_addr", "TEXT").await?;
    }
    if from < 2 {
        // v2: add sse_kind column to session_events for lossless event-kind
        // replay. The column is nullable so existing rows stay valid.
        add_column_if_absent(conn, "session_events", "sse_kind", "TEXT").await?;
    }
    if from < 3 {
        // v3: plan→act handoff boundary + active skill on sessions, so resume
        // can reconstruct the post-handoff focused transcript and the active
        // skill across restarts. All nullable so existing rows stay valid.
        add_column_if_absent(conn, "sessions", "handoff_seq", "INTEGER").await?;
        add_column_if_absent(conn, "sessions", "handoff_plan", "TEXT").await?;
        add_column_if_absent(conn, "sessions", "skill", "TEXT").await?;
    }
    if from < 4 {
        // v4: image attachments on session inputs (multimodal prompts). The
        // column is a JSON array of data URIs, defaulting to an empty array so
        // existing plain-text rows stay valid. NOT NULL + DEFAULT keeps the
        // invariant that the column is always readable as JSON.
        add_column_if_absent(
            conn,
            "session_inputs",
            "images_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )
        .await?;
    }
    if from < 5 {
        // v5: task_type column on sessions distinguishes parent (top-level)
        // sessions from subagent child sessions. NOT NULL with a default of
        // 'parent' so existing rows are valid parents. Backfill any rows that
        // are already linked as subagent children, then create the filter
        // index. (CREATE TABLE already carries the column for fresh DBs, so
        // add_column_if_absent keeps this idempotent.)
        add_column_if_absent(
            conn,
            "sessions",
            "task_type",
            "TEXT NOT NULL DEFAULT 'parent'",
        )
        .await?;
        conn.execute(
            "UPDATE sessions SET task_type = 'subagent' WHERE id IN (SELECT child_session_id FROM subagent_tasks)",
            (),
        )
        .await
        .context("backfill task_type")?;
        conn.execute(CREATE_INDEX_SESSION_TASK_TYPE, ()).await?;
    }
    if from < 6 {
        // v6: display-only text on session inputs, preserving the verbatim
        // original (which may contain the `$skill` token) so the TUI queue/
        // steer panel can restore it after resume/reload. `prompt` keeps the
        // clean token-stripped text that the LLM consumes; this column is
        // never fed to the LLM. Nullable so pre-existing rows stay valid —
        // old rows keep NULL and display layers fall back to `prompt`.
        add_column_if_absent(conn, "session_inputs", "display_text", "TEXT").await?;
    }
    if from < 7 {
        // v7: image URLs preserved across compaction, persisted as
        // `summary_images_json` so resume can rebuild the synthetic
        // summary message without reloading the soft-deleted
        // compacted head. Nullable so existing rows stay valid.
        add_column_if_absent(conn, "sessions", "summary_images_json", "TEXT").await?;
    }
    if from < 8 {
        // v8: requirement column on sessions, persisting the task description
        // text edited via the /requirement slash command so it survives resume.
        add_column_if_absent(conn, "sessions", "requirement", "TEXT").await?;
    }
    if from < 9 {
        conn.execute(CREATE_TODO_WORKFLOWS, ()).await?;
        conn.execute(CREATE_TODO_ITEMS, ()).await?;
        conn.execute(CREATE_TODO_EVENTS, ()).await?;
        conn.execute(CREATE_INDEX_TODO_STATUS, ()).await?;
        conn.execute(CREATE_INDEX_TODO_EVENTS, ()).await?;
    }
    if from < 10 {
        // v10: plan-phase persistence. `plan_snapshot` preserves the final
        // plan text across compaction so plan->act handoff still finds it;
        // `plan_input_count` re-arms plan-phase affordances after resume.
        add_column_if_absent(conn, "sessions", "plan_snapshot", "TEXT").await?;
        add_column_if_absent(
            conn,
            "sessions",
            "plan_input_count",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
    }
    if from < 10 {
        // v10: `recorded` marks a promoted input as durably consumed (written
        // into the transcript or applied as a control command). A promoted row
        // with recorded=0 is an orphan (crash / hard-cancel between promote
        // and consume) that `recover_orphan_inputs` can flip back to pending.
        add_column_if_absent(
            conn,
            "session_inputs",
            "recorded",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        // One-time backfill: rows already promoted when the column lands
        // predate the marker and are historical audit rows already reflected
        // in the transcript, so treat them as consumed. Pending rows keep
        // recorded=0.
        conn.execute(
            "UPDATE session_inputs SET recorded = 1 WHERE promoted_seq IS NOT NULL AND recorded = 0",
            (),
        )
        .await
        .context("backfill recorded")?;
    }
    if from < 11 {
        // v11: session-scoped autopilot mode for the `/ap` "session-only"
        // switch. NULL = follow the global config; "off"/"ap"/"review" pins
        // this session's mode so resume honors it (same role as `model`).
        add_column_if_absent(conn, "sessions", "autopilot_mode", "TEXT").await?;
    }
    if from < 12 {
        // v12: multi-node distributed execution plane — the worker-node
        // registry plus its dispatch queue. Both statements are `CREATE IF NOT
        // EXISTS`: fresh databases already carry them from bootstrap's CREATE
        // batch, older databases create them here. No existing rows need
        // backfilling (the tables are new), so the upgrade is a no-op beyond
        // the DDL.
        conn.execute(CREATE_NODES, ()).await?;
        conn.execute(CREATE_NODE_TASKS, ()).await?;
    }
    if from < 28 {
        // v28: lane tagging. Operator executions stamp their runtime.db
        // sessions with `kind` so the control-plane swimlanes (and the
        // default chat list) filter at the store layer instead of relying on
        // `id`-prefix/title conventions. Old rows stay NULL and keep their
        // existing fallback resolution.
        add_column_if_absent(conn, "sessions", "kind", "TEXT").await?;
    }
    if from < 26 {
        // v26: control-plane cron scheduler fire ledger. CREATE IF NOT
        // EXISTS keeps this idempotent; the table is new (append-only
        // history), so no rows need backfilling.
        conn.execute(CREATE_SCHEDULE_RUNS, ()).await?;
    }
    if from < 27 {
        // v27: schedule definitions move into the DB (the cron scheduler's
        // source of truth; `schedules.json` degrades to a bootstrap seed).
        // CREATE IF NOT EXISTS keeps this idempotent; the table is new, so
        // no rows need backfilling. No FK to schedule_runs: the fire ledger
        // outlives its definitions by design.
        conn.execute(CREATE_SCHEDULES, ()).await?;
    }
    if from < 24 {
        // v24: platform users. CREATE IF NOT EXISTS keeps this idempotent;
        // no extra index needed (UNIQUE constraints carry the lookups).
        conn.execute(CREATE_PLATFORM_USERS, ()).await?;
    }
    if from < 23 {
        project_relations::migrate(conn).await?;
    }

    Ok(())
}
