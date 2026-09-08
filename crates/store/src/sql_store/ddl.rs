//! Idempotent project-table DDL in MySQL dialect, mirroring the libsql schema
//! (`libsql_store::schema`'s `project_*` consts) column-for-column:
//! ids `VARCHAR(64)`, titles `VARCHAR(512) NOT NULL`, long markdown columns
//! `MEDIUMTEXT` (`STRING` on StarRocks, which has no MEDIUMTEXT), status/kind
//! `VARCHAR(32) NOT NULL`, agent `VARCHAR(64) NOT NULL`, session ids
//! `VARCHAR(64) NULL`, numeric columns `BIGINT NOT NULL` (`finished_at`
//! NULL), executor names/refs `VARCHAR(255) NULL`, inline executor specs
//! `{text}`. No FK constraints, same policy as libsql — the cascades are
//! explicit code so every backend behaves identically.

use anyhow::{Context, Result};
use sqlx::{MySqlPool, Row};

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
  updated_at BIGINT NOT NULL,
  executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent',
  executor_ref VARCHAR(255) NULL,
  executor_spec {text} NULL";

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
  created_at BIGINT NOT NULL,
  executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent',
  capability_id VARCHAR(64) NULL,
  plan_id VARCHAR(64) NULL,
  output_ref VARCHAR(255) NULL,
  input_snapshot {text} NULL,
  trace_manifest {text} NULL";

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

/// Columns added after the initial table shape (the executor dimension):
/// applied only when `information_schema.columns` reports them missing,
/// so existing deployments upgrade in place and fresh ones no-op.
/// Definitions MUST mirror the CREATE TABLE column consts above — the
/// drift test below enforces the names line up.
const UPGRADE_COLUMNS: &[(&str, &[&str])] = &[
    (
        "project_todos",
        &[
            "executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent'",
            "executor_ref VARCHAR(255) NULL",
            "executor_spec {text} NULL",
        ],
    ),
    (
        "project_todo_runs",
        &[
            "executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent'",
            "capability_id VARCHAR(64) NULL",
            "plan_id VARCHAR(64) NULL",
            "output_ref VARCHAR(255) NULL",
            "input_snapshot {text} NULL",
            "trace_manifest {text} NULL",
        ],
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

/// First whitespace-separated token of a column definition — the column's
/// name, matched against `information_schema.columns` entries.
fn column_name(def: &str) -> &str {
    def.split_whitespace().next().unwrap_or("")
}

/// The `ALTER TABLE ... ADD COLUMN` statement for one missing column, with
/// `{text}` resolved per backend exactly like `create_table` does.
fn add_column_sql(table: &str, col_def: &str, starrocks: bool) -> String {
    format!(
        "ALTER TABLE {table} ADD COLUMN {}",
        col_def.replace("{text}", text_type(starrocks))
    )
}

/// Pure decision core of [`upgrade`]: the column defs whose names are NOT in
/// the existing column set (names compared case-insensitively — MySQL and
/// StarRocks report `information_schema.columns` names in different cases).
/// An empty result is the idempotent no-op that fresh deployments — created
/// by `apply` with the columns already in place — always hit.
fn missing_columns<'a>(cols: &[&'a str], existing: &[String]) -> Vec<&'a str> {
    cols.iter()
        .copied()
        .filter(|col| {
            let name = column_name(col).to_lowercase();
            !existing.iter().any(|c| c.to_lowercase() == name)
        })
        .collect()
}

/// In-place upgrade for deployments whose tables predate the executor
/// columns: `apply` only ever runs `CREATE TABLE IF NOT EXISTS`, which is a
/// no-op on an existing table, so writes would fail with `Unknown column`.
/// For each [`UPGRADE_COLUMNS`] entry, ask `information_schema.columns`
/// (exposed by both MySQL and StarRocks) what the table already has and
/// `ADD COLUMN` only the missing definitions.
pub async fn upgrade(pool: &MySqlPool, starrocks: bool) -> Result<()> {
    for (table, cols) in UPGRADE_COLUMNS {
        // The column listing rides the shared read helper: on StarRocks
        // every statement (reads included) must take the text protocol —
        // cached prepared SELECTs were observed returning stale snapshots,
        // which here could re-ALTER a column added moments ago.
        let rows = super::exec_read_all(
            pool,
            starrocks,
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ?",
            &[super::Arg::Text((*table).to_string())],
        )
        .await
        .with_context(|| format!("upgrade table {table}: list columns"))?;
        let existing: Vec<String> = rows
            .iter()
            .map(|r| r.try_get::<String, _>(0))
            .collect::<std::result::Result<_, _>>()
            .with_context(|| format!("upgrade table {table}: read column_name"))?;
        for col in missing_columns(cols, &existing) {
            let sql = add_column_sql(table, col, starrocks);
            // Same prepared-vs-text split as `apply`: MySQL runs DDL through
            // the prepared path, StarRocks DDL must ride the text protocol.
            let res = if starrocks {
                sqlx::raw_sql(&sql).execute(pool).await
            } else {
                sqlx::query(&sql).execute(pool).await
            };
            res.with_context(|| format!("upgrade table {table}: add {col}"))?;
        }
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

    #[test]
    fn upgrade_alters_carry_backend_text_type_and_full_definitions() {
        for (table, cols) in UPGRADE_COLUMNS {
            for col in *cols {
                let mysql = add_column_sql(table, col, false);
                let starrocks = add_column_sql(table, col, true);
                assert!(!mysql.contains("{text}") && !starrocks.contains("{text}"));
                // The whole definition travels verbatim with only `{text}`
                // resolved per backend — a NOT NULL DEFAULT column keeps its
                // full clause, a `{text}` column switches type.
                assert!(
                    mysql.ends_with(&format!(
                        "ADD COLUMN {}",
                        col.replace("{text}", "MEDIUMTEXT")
                    )),
                    "{mysql}"
                );
                assert!(
                    starrocks.ends_with(&format!("ADD COLUMN {}", col.replace("{text}", "STRING"))),
                    "{starrocks}"
                );
            }
        }
        // The executor-spec column is the one that actually switches type.
        let spec = "executor_spec {text} NULL";
        assert!(add_column_sql("project_todos", spec, false)
            .contains("ADD COLUMN executor_spec MEDIUMTEXT NULL"));
        assert!(add_column_sql("project_todos", spec, true)
            .contains("ADD COLUMN executor_spec STRING NULL"));
        // NOT NULL DEFAULT columns keep the whole clause.
        let kind = "executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent'";
        assert!(add_column_sql("project_todo_runs", kind, false)
            .contains("ADD COLUMN executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent'"));
    }

    #[test]
    fn upgrade_columns_do_not_drift_from_create_table_consts() {
        for (table, cols) in UPGRADE_COLUMNS {
            let (_, create_cols, _) = TABLES
                .iter()
                .find(|(name, _, _)| name == table)
                .unwrap_or_else(|| panic!("upgrade table {table} missing from TABLES"));
            // Exact-token match on the CREATE column list: every upgrade
            // column must already exist there by name, so the two lists can
            // never diverge (fresh CREATE and old-table ALTER agree).
            let create_names: Vec<&str> = create_cols
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .collect();
            for col in *cols {
                let name = column_name(col);
                assert!(
                    create_names.contains(&name),
                    "{table}: upgrade column {name} absent from CREATE TABLE const"
                );
            }
        }
    }

    #[test]
    fn column_name_extracts_first_whitespace_token() {
        assert_eq!(
            column_name("executor_kind VARCHAR(32) NOT NULL DEFAULT 'agent'"),
            "executor_kind"
        );
        assert_eq!(column_name("executor_spec {text} NULL"), "executor_spec");
        assert_eq!(column_name(""), "");
    }

    #[test]
    fn missing_columns_drives_idempotent_upgrade_shape() {
        let (todos, todo_cols) = &UPGRADE_COLUMNS[0];
        assert_eq!(*todos, "project_todos");
        // Everything already present (fresh deployment after `apply`):
        // nothing to ALTER — the idempotent no-op.
        let existing: Vec<String> = todo_cols
            .iter()
            .map(|c| column_name(c).to_lowercase())
            .collect();
        assert!(missing_columns(todo_cols, &existing).is_empty());
        // Pre-executor table: only the executor columns are missing, in order.
        let pre_executor: Vec<String> = vec![
            "id".into(),
            "milestone_id".into(),
            "title".into(),
            "EXECUTOR_KIND".into(), // case-insensitive match, as StarRocks
            "executor_ref".into(),  // reports mixed-case names
        ];
        assert_eq!(
            missing_columns(todo_cols, &pre_executor),
            vec!["executor_spec {text} NULL"]
        );
    }
}
