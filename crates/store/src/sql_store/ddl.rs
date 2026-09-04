//! Idempotent project-table DDL in MySQL dialect, mirroring the libsql schema
//! (`libsql_store::schema`'s `project_*` consts) column-for-column:
//! ids `VARCHAR(64)`, titles `VARCHAR(512) NOT NULL`, long markdown columns
//! `MEDIUMTEXT` (`STRING` on StarRocks, which has no MEDIUMTEXT), status/kind
//! `VARCHAR(32) NOT NULL`, agent `VARCHAR(64) NOT NULL`, session ids
//! `VARCHAR(64) NULL`, numeric columns `BIGINT NOT NULL` (`finished_at`
//! NULL). No FK constraints, same policy as libsql — the cascades are
//! explicit code so every backend behaves identically.

use anyhow::{Context, Result};
use sqlx::MySqlPool;

const GOAL_COLUMNS: &str = "\
  id VARCHAR(64) NOT NULL,
  title VARCHAR(512) NOT NULL,
  detail_md {text} NULL,
  status VARCHAR(32) NOT NULL,
  sort_key BIGINT NOT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL";

const MILESTONE_COLUMNS: &str = "\
  id VARCHAR(64) NOT NULL,
  goal_id VARCHAR(64) NOT NULL,
  title VARCHAR(512) NOT NULL,
  detail_md {text} NULL,
  status VARCHAR(32) NOT NULL,
  sort_key BIGINT NOT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL";

const TODO_COLUMNS: &str = "\
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

const RUN_COLUMNS: &str = "\
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

/// `(table, columns, secondary-index clause)`; the index clause is MySQL-only.
const TABLES: &[(&str, &str, &str)] = &[
    ("project_goals", GOAL_COLUMNS, ""),
    (
        "project_milestones",
        MILESTONE_COLUMNS,
        "KEY idx_project_milestones_goal (goal_id)",
    ),
    (
        "project_todos",
        TODO_COLUMNS,
        "KEY idx_project_todos_milestone (milestone_id)",
    ),
    (
        "project_todo_runs",
        RUN_COLUMNS,
        "KEY idx_project_todo_runs_todo (todo_id, version)",
    ),
];

/// Long-markdown column type per backend: MySQL's 16MB `MEDIUMTEXT`, or
/// StarRocks' unbounded `STRING` (StarRocks has no MEDIUMTEXT and caps
/// VARCHAR at 1MB).
fn text_type(starrocks: bool) -> &'static str {
    if starrocks {
        "STRING"
    } else {
        "MEDIUMTEXT"
    }
}

fn create_table(name: &str, columns: &str, index: &str, starrocks: bool) -> String {
    let columns = &columns.replace("{text}", text_type(starrocks));
    if starrocks {
        // StarRocks: the primary-key table model (INSERT upserts on duplicate
        // id, UPDATE/DELETE supported) — its grammar takes `PRIMARY KEY(id)`
        // as a clause AFTER the column list (an inline constraint inside the
        // parens is a syntax error there), plus the mandatory distribution
        // clause. Secondary indexes are NOT declared: StarRocks CREATE TABLE
        // has no plain secondary-INDEX clause (only bitmap/bloomfilter index
        // properties), the project tables are tiny, and full scans are fine.
        format!(
            "CREATE TABLE IF NOT EXISTS {name} (\n{columns}\n) \
             PRIMARY KEY (id)\nDISTRIBUTED BY HASH(id) BUCKETS 1"
        )
    } else {
        let index = if index.is_empty() {
            String::new()
        } else {
            format!(",\n  {index}")
        };
        format!(
            "CREATE TABLE IF NOT EXISTS {name} (\n{columns},\n  PRIMARY KEY (id){index}\n) \
             ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
        )
    }
}

/// Apply the four `CREATE TABLE IF NOT EXISTS` statements sequentially.
pub async fn apply(pool: &MySqlPool, starrocks: bool) -> Result<()> {
    for (name, columns, index) in TABLES {
        let sql = create_table(name, columns, index, starrocks);
        // MySQL: the plain `query` path (prepared) as everywhere else.
        // StarRocks: DDL is not supported through the prepared-statement
        // protocol (ER_UNSUPPORTED_PS), so DDL goes over the text protocol.
        let res = if starrocks {
            sqlx::raw_sql(&sql).execute(pool).await
        } else {
            sqlx::query(&sql).execute(pool).await
        };
        res.with_context(|| format!("create table {name}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_stmts_carry_engine_charset_and_indexes() {
        let goals = create_table(TABLES[0].0, TABLES[0].1, TABLES[0].2, false);
        assert!(goals.contains("PRIMARY KEY (id)"));
        assert!(goals.ends_with("ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"));
        assert!(
            !goals.contains("KEY idx_"),
            "goals table has no secondary index"
        );

        let runs = create_table(TABLES[3].0, TABLES[3].1, TABLES[3].2, false);
        assert!(runs.contains("KEY idx_project_todo_runs_todo (todo_id, version)"));
    }

    #[test]
    fn starrocks_stmts_carry_pk_model_and_distribution_but_no_indexes() {
        for (name, columns, index) in TABLES {
            let sql = create_table(name, columns, index, true);
            assert!(sql.contains(") PRIMARY KEY (id)"), "{name}");
            assert!(sql.ends_with("DISTRIBUTED BY HASH(id) BUCKETS 1"), "{name}");
            assert!(!sql.contains("KEY idx_"), "no secondary indexes on {name}");
            assert!(!sql.contains("ENGINE="), "{name}");
        }
    }
}
