use opencoder_core::{StorageBackend, StorageConfig};
use opencoder_store::sql_store;
use opencoder_store::{
    ProjectExecutorKind, ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunKind,
    ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus, ProjectTodoStatus,
};
use sqlx::mysql::MySqlConnectOptions;
use sqlx::{MySqlPool, Row};
use std::future::Future;
use std::time::Duration;
fn env_dsn(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
async fn eventually<F, Fut>(what: &str, mut probe: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..100 {
        if probe().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}
async fn connect(dsn: &str, starrocks: bool) -> MySqlPool {
    let mut options = dsn.parse::<MySqlConnectOptions>().expect("parse test DSN");
    if starrocks {
        options = options
            .pipes_as_concat(false)
            .no_engine_substitution(false)
            .timezone(None)
            .set_names(false);
    }
    MySqlPool::connect_with(options)
        .await
        .expect("connect test DSN")
}
async fn table_columns(pool: &MySqlPool, starrocks: bool, table: &str) -> Vec<String> {
    let rows = if starrocks {
        let sql = format!(
            "SELECT column_name FROM information_schema.columns \
               WHERE table_schema = DATABASE() AND table_name = '{table}'"
        );
        sqlx::raw_sql(&sql).fetch_all(pool).await
    } else {
        sqlx::query(
            "SELECT column_name FROM information_schema.columns \
               WHERE table_schema = DATABASE() AND table_name = ?",
        )
        .bind(table)
        .fetch_all(pool)
        .await
    }
    .expect("list table columns");
    rows.iter()
        .map(|r| {
            r.try_get::<String, _>(0)
                .expect("read column_name")
                .to_lowercase()
        })
        .collect()
}
async fn drop_tables(pool: &MySqlPool) {
    for table in ["project_todos", "project_todo_runs"] {
        sqlx::raw_sql(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(pool)
            .await
            .expect("drop table");
    }
}
fn has(cols: &[String], name: &str) -> bool {
    cols.iter().any(|c| c == name)
}
const LEGACY_TODO_COLUMNS: &str = "\
id VARCHAR(64) NOT NULL,
milestone_id VARCHAR(64) NULL,
title VARCHAR(512) NOT NULL,
draft {text} NOT NULL,
plan_md {text} NULL,
status VARCHAR(32) NOT NULL,
agent VARCHAR(64) NOT NULL,
active_session_id VARCHAR(64) NULL,
created_at BIGINT NOT NULL,
updated_at BIGINT NOT NULL";
const LEGACY_RUN_COLUMNS: &str = "\
id VARCHAR(64) NOT NULL,
todo_id VARCHAR(64) NOT NULL,
kind VARCHAR(32) NOT NULL,
version BIGINT NOT NULL,
plan_md {text} NULL,
output_md {text} NULL,
agent VARCHAR(64) NOT NULL,
session_id VARCHAR(64) NULL,
status VARCHAR(32) NOT NULL,
started_at BIGINT NOT NULL,
finished_at BIGINT NULL,
created_at BIGINT NOT NULL";
fn legacy_create(table: &str, columns: &str, index: &str, starrocks: bool) -> String {
    let text = if starrocks { "STRING" } else { "MEDIUMTEXT" };
    let columns = columns.replace("{text}", text);
    if starrocks {
        format!(
            "CREATE TABLE IF NOT EXISTS {table} (\n{columns}\n) \
               PRIMARY KEY (id)\nDISTRIBUTED BY HASH(id) BUCKETS 1"
        )
    } else {
        let index = if index.is_empty() {
            String::new()
        } else {
            format!(",\n  {index}")
        };
        format!(
            "CREATE TABLE IF NOT EXISTS {table} (\n{columns},\n  PRIMARY KEY (id){index}\n) \
               ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
        )
    }
}
fn storage(backend: StorageBackend, dsn: &str) -> StorageConfig {
    StorageConfig {
        backend,
        mysql: (backend == StorageBackend::Mysql).then_some(dsn.to_string()),
        starrocks: (backend == StorageBackend::Starrocks).then_some(dsn.to_string()),
    }
}
async fn upgrade_contract(backend: StorageBackend, dsn: &str) {
    let starrocks = backend == StorageBackend::Starrocks;
    let pool = connect(dsn, starrocks).await;
    drop_tables(&pool).await;
    let legacy_tables: &[(&str, &str, &str)] = &[
        (
            "project_todos",
            LEGACY_TODO_COLUMNS,
            "KEY idx_project_todos_milestone (milestone_id)",
        ),
        (
            "project_todo_runs",
            LEGACY_RUN_COLUMNS,
            "KEY idx_project_todo_runs_todo (todo_id, version)",
        ),
    ];
    for (table, columns, index) in legacy_tables {
        let sql = legacy_create(table, columns, index, starrocks);
        sqlx::raw_sql(&sql)
            .execute(&pool)
            .await
            .expect("create legacy table");
    }
    sqlx::raw_sql("INSERT INTO project_todos (id, title, draft, status, agent, created_at, updated_at) VALUES ('legacy-board', 'Legacy', 'task', 'running', 'act', 1234, 1234)")
          .execute(&pool)
          .await
          .expect("insert legacy TODO");
    for column in [
        "board_status VARCHAR(32) NOT NULL DEFAULT 'backlog'",
        "position BIGINT NOT NULL DEFAULT -1",
    ] {
        sqlx::raw_sql(&format!("ALTER TABLE project_todos ADD COLUMN {column}"))
            .execute(&pool)
            .await
            .expect("simulate interrupted board upgrade");
    }
    let todos = table_columns(&pool, starrocks, "project_todos").await;
    let runs = table_columns(&pool, starrocks, "project_todo_runs").await;
    for col in ["executor_kind", "executor_ref", "executor_spec"] {
        assert!(!has(&todos, col), "legacy project_todos lacks {col}");
    }
    for col in ["executor_kind", "capability_id", "plan_id", "output_ref"] {
        assert!(!has(&runs, col), "legacy project_todo_runs lacks {col}");
    }
    let p = sql_store::open(&storage(backend, dsn))
        .await
        .expect("open upgrades legacy tables in place");
    eventually("interrupted board upgrade resumes", || async {
          matches!(p.get_todo("legacy-board").await, Ok(Some(todo)) if todo.board_status == "in_progress" && todo.position == 1234)
      })
      .await;
    sqlx::raw_sql(
        "UPDATE project_todos SET board_status = 'todo', position = 0 WHERE id = 'legacy-board'",
    )
    .execute(&pool)
    .await
    .expect("set intentional board position");
    let reopened = sql_store::open(&storage(backend, dsn))
        .await
        .expect("repeat upgrade must preserve the board");
    eventually("intentional zero position survives restart", || async {
          matches!(reopened.get_todo("legacy-board").await, Ok(Some(todo)) if todo.board_status == "todo" && todo.position == 0)
      })
      .await;
    eventually("upgrade columns present on both tables", || async {
        let todos = table_columns(&pool, starrocks, "project_todos").await;
        let runs = table_columns(&pool, starrocks, "project_todo_runs").await;
        ["executor_kind", "executor_ref", "executor_spec"]
            .iter()
            .all(|c| has(&todos, c))
            && ["executor_kind", "capability_id", "plan_id", "output_ref"]
                .iter()
                .all(|c| has(&runs, c))
    })
    .await;
    let uniq = ulid::Ulid::new().to_string();
    let ts = 3_000i64;
    let (todo, run) = (format!("todo-{uniq}"), format!("run-{uniq}"));
    let spec = r#"{"members":[]}"#;
    p.create_todo(&ProjectTodoRecord {
        id: todo.clone(),
        initiative_id: None,
        title: "legacy todo".into(),
        draft: "draft".into(),
        plan_md: None,
        status: ProjectTodoStatus::Planned,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Team,
        executor_ref: Some("team-a".into()),
        executor_spec: Some(spec.into()),
        active_session_id: None,
        board_status: "backlog".into(),
        position: 0,
        capability_id: None,
        created_at: ts,
        updated_at: ts,
    })
    .await
    .unwrap();
    eventually("todo executor fields round-trip", || async {
        matches!(
            p.get_todo(&todo).await,
            Ok(Some(t)) if t.executor_kind == ProjectExecutorKind::Team
                && t.executor_ref.as_deref() == Some("team-a")
                && t.executor_spec.as_deref() == Some(spec)
        )
    })
    .await;
    assert!(p
        .patch_todo(
            &todo,
            &ProjectTodoPatch {
                executor_ref: Some(Some("team-b".into())),
                executor_spec: Some(None),
                ..Default::default()
            },
            ts + 1,
        )
        .await
        .unwrap());
    eventually("todo executor patch visible", || async {
        p.get_todo(&todo)
            .await
            .unwrap()
            .map(|t| t.executor_ref.as_deref() == Some("team-b") && t.executor_spec.is_none())
            .unwrap_or(false)
    })
    .await;
    let v1 = p.next_todo_version(&todo).await.unwrap();
    assert_eq!(v1, 1, "upgraded todo still starts at version 1");
    let (cap, plan, out) = (
        format!("cap-{uniq}"),
        format!("plan-{uniq}"),
        "dag://runs/1/artifacts".to_string(),
    );
    p.create_todo_run(&ProjectTodoRunRecord {
        input_snapshot: None,
        trace_manifest: None,
        id: run.clone(),
        todo_id: todo.clone(),
        kind: ProjectTodoRunKind::Execute,
        version: v1,
        plan_md: None,
        output_md: None,
        agent: "act".into(),
        executor_kind: ProjectExecutorKind::Dag,
        capability_id: Some(cap.clone()),
        plan_id: Some(plan.clone()),
        output_ref: Some(out.clone()),
        session_id: None,
        status: ProjectTodoRunStatus::Running,
        started_at: ts + 2,
        finished_at: None,
        created_at: ts + 2,
    })
    .await
    .unwrap();
    eventually("run executor fields round-trip", || async {
        p.get_todo_run(&run)
            .await
            .unwrap()
            .map(|r| {
                r.executor_kind == ProjectExecutorKind::Dag
                    && r.capability_id.as_deref() == Some(cap.as_str())
                    && r.plan_id.as_deref() == Some(plan.as_str())
                    && r.output_ref.as_deref() == Some(out.as_str())
            })
            .unwrap_or(false)
    })
    .await;
    assert!(p
        .patch_todo_run(
            &run,
            &ProjectTodoRunPatch {
                output_ref: Some("dag://runs/2/artifacts".into()),
                ..Default::default()
            },
            ts + 3,
        )
        .await
        .unwrap());
    eventually("run output_ref patch visible", || async {
        p.get_todo_run(&run)
            .await
            .unwrap()
            .map(|r| r.output_ref.as_deref() == Some("dag://runs/2/artifacts"))
            .unwrap_or(false)
    })
    .await;
    assert!(p.delete_todo(&todo).await.unwrap());
    eventually("todo + runs gone", || async {
        p.get_todo(&todo).await.unwrap().is_none()
            && p.list_todo_runs(&todo).await.unwrap().is_empty()
    })
    .await;
    sql_store::open(&storage(backend, dsn))
        .await
        .expect("second open is an idempotent upgrade no-op");
    for table in ["project_todos", "project_todo_runs"] {
        sqlx::raw_sql(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(&pool)
            .await
            .expect("cleanup drop table");
    }
    pool.close().await;
}
#[tokio::test]
async fn mysql_project_upgrade_contract() {
    let Some(dsn) = env_dsn("OC_TEST_MYSQL_DSN") else {
        eprintln!("warning: OC_TEST_MYSQL_DSN not set — skipping sql_store integration test");
        return;
    };
    upgrade_contract(StorageBackend::Mysql, &dsn).await;
}
#[cfg(feature = "starrocks")]
#[tokio::test]
async fn starrocks_project_upgrade_contract() {
    let Some(dsn) = env_dsn("OC_TEST_STARROCKS_DSN") else {
        eprintln!("warning: OC_TEST_STARROCKS_DSN not set — skipping sql_store integration test");
        return;
    };
    upgrade_contract(StorageBackend::Starrocks, &dsn).await;
}
