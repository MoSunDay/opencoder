//! Optional MySQL / StarRocks backends for the project tables (goals /
//! milestones / todos / runs), feature-gated behind `mysql` / `starrocks`.
//!
//! StarRocks speaks the MySQL wire protocol, so both ride the same sqlx
//! mysql driver; the `starrocks` flag only branches DDL (primary-key table
//! model, no secondary indexes) and delete cascades (no cross-statement
//! transactions there). Untyped `sqlx::query` only — the macros feature is
//! not enabled, so no compile-time database is ever required.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use opencoder_core::{StorageBackend, StorageConfig};
use sqlx::mysql::{MySqlArguments, MySqlConnectOptions};
use sqlx::{MySqlPool, Row};

use crate::project::ProjectStore;
use crate::project_types::{
    ProjectGoalPatch, ProjectGoalRecord, ProjectMilestonePatch, ProjectMilestoneRecord,
    ProjectTodoPatch, ProjectTodoRecord, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};

pub mod ddl;
mod project_crud;
mod project_crud_runs;

/// A pooled MySQL/StarRocks project store. The pool is cheap to clone and
/// the struct is stateless besides the `starrocks` behavior flag.
pub struct SqlProjectStore {
    pool: MySqlPool,
    starrocks: bool,
}

/// Connect to the configured backend, apply the idempotent project DDL, and
/// return the store behind the `Arc<dyn ProjectStore>` seam.
pub async fn open(storage: &StorageConfig) -> Result<Arc<dyn ProjectStore>> {
    let starrocks = match storage.backend {
        StorageBackend::Mysql => false,
        StorageBackend::Starrocks => true,
        StorageBackend::Libsql => anyhow::bail!("libsql backend does not go through sql_store"),
    };
    let backend = storage.backend.as_str();
    let field = if starrocks { "starrocks" } else { "mysql" };
    // `dsn()` picks this backend's field AND expands `{VAR}` env refs, so
    // credentials never need to land in the config file.
    let dsn = storage.dsn().ok_or_else(|| {
        anyhow::anyhow!(
            "storage backend '{backend}' requires a DSN \
             (config storage.{field} / env {{VAR}} expansion)"
        )
    })?;
    let mut options = dsn
        .parse::<MySqlConnectOptions>()
        .with_context(|| format!("parse {backend} DSN"))?;
    if starrocks {
        // StarRocks' SET grammar only accepts constant expressions, but
        // sqlx's default session setup appends sql_mode via a
        // `(SELECT CONCAT(@@sql_mode, ...))` subquery — StarRocks rejects
        // that at handshake ("Set statement only support constant expr").
        // Worse, StarRocks does not support `SET` through the prepared
        // statement protocol at all (ER_UNSUPPORTED_PS), so the safest
        // handshake is none: no sql_mode tweak, no time_zone (we never read
        // TIMESTAMP columns — all times are BIGINT ms), no SET NAMES (the
        // wire stays utf8mb4 either way). MySQL keeps sqlx's defaults.
        options = options
            .pipes_as_concat(false)
            .no_engine_substitution(false)
            .timezone(None)
            .set_names(false);
    }
    let pool = sqlx::MySqlPool::connect_with(options)
        .await
        .with_context(|| format!("connect {backend}"))?;
    ddl::apply(&pool, starrocks).await?;
    Ok(Arc::new(SqlProjectStore { pool, starrocks }))
}

#[async_trait]
impl ProjectStore for SqlProjectStore {
    fn project_backend_name(&self) -> &'static str {
        if self.starrocks {
            "starrocks"
        } else {
            "mysql"
        }
    }

    async fn create_goal(&self, rec: &ProjectGoalRecord) -> Result<()> {
        project_crud::create_goal(&self.pool, self.starrocks, rec).await
    }
    async fn patch_goal(&self, id: &str, patch: &ProjectGoalPatch, now_ms: i64) -> Result<bool> {
        project_crud::patch_goal(&self.pool, self.starrocks, id, patch, now_ms).await
    }
    async fn delete_goal(&self, id: &str) -> Result<bool> {
        project_crud::delete_goal(&self.pool, self.starrocks, id).await
    }
    async fn list_goals(&self) -> Result<Vec<ProjectGoalRecord>> {
        project_crud::list_goals(&self.pool, self.starrocks).await
    }

    async fn create_milestone(&self, rec: &ProjectMilestoneRecord) -> Result<()> {
        project_crud::create_milestone(&self.pool, self.starrocks, rec).await
    }
    async fn patch_milestone(
        &self,
        id: &str,
        patch: &ProjectMilestonePatch,
        now_ms: i64,
    ) -> Result<bool> {
        project_crud::patch_milestone(&self.pool, self.starrocks, id, patch, now_ms).await
    }
    async fn delete_milestone(&self, id: &str) -> Result<bool> {
        project_crud::delete_milestone(&self.pool, self.starrocks, id).await
    }
    async fn list_milestones(&self, goal_id: Option<&str>) -> Result<Vec<ProjectMilestoneRecord>> {
        project_crud::list_milestones(&self.pool, self.starrocks, goal_id).await
    }

    async fn create_todo(&self, rec: &ProjectTodoRecord) -> Result<()> {
        project_crud_runs::create_todo(&self.pool, self.starrocks, rec).await
    }
    async fn patch_todo(&self, id: &str, patch: &ProjectTodoPatch, now_ms: i64) -> Result<bool> {
        project_crud_runs::patch_todo(&self.pool, self.starrocks, id, patch, now_ms).await
    }
    async fn claim_todo_running(&self, id: &str, now_ms: i64) -> Result<bool> {
        project_crud_runs::claim_todo_running(&self.pool, self.starrocks, id, now_ms).await
    }
    async fn patch_todo_when(
        &self,
        id: &str,
        when: ProjectTodoStatus,
        patch: &ProjectTodoPatch,
        now_ms: i64,
    ) -> Result<bool> {
        project_crud_runs::patch_todo_when(&self.pool, self.starrocks, id, when, patch, now_ms)
            .await
    }
    async fn delete_todo(&self, id: &str) -> Result<bool> {
        project_crud_runs::delete_todo(&self.pool, self.starrocks, id).await
    }
    async fn get_todo(&self, id: &str) -> Result<Option<ProjectTodoRecord>> {
        project_crud_runs::get_todo(&self.pool, self.starrocks, id).await
    }
    async fn list_todos(&self, milestone_id: Option<&str>) -> Result<Vec<ProjectTodoRecord>> {
        project_crud_runs::list_todos(&self.pool, self.starrocks, milestone_id).await
    }

    async fn create_todo_run(&self, rec: &ProjectTodoRunRecord) -> Result<()> {
        project_crud_runs::create_todo_run(&self.pool, self.starrocks, rec).await
    }
    async fn patch_todo_run(
        &self,
        id: &str,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        project_crud_runs::patch_todo_run(&self.pool, self.starrocks, id, patch, now_ms).await
    }
    async fn patch_todo_run_when(
        &self,
        id: &str,
        when: ProjectTodoRunStatus,
        patch: &ProjectTodoRunPatch,
        now_ms: i64,
    ) -> Result<bool> {
        project_crud_runs::patch_todo_run_when(&self.pool, self.starrocks, id, when, patch, now_ms)
            .await
    }
    async fn get_todo_run(&self, id: &str) -> Result<Option<ProjectTodoRunRecord>> {
        project_crud_runs::get_todo_run(&self.pool, self.starrocks, id).await
    }
    async fn list_todo_runs(&self, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
        project_crud_runs::list_todo_runs(&self.pool, self.starrocks, todo_id).await
    }
    async fn list_running_todo_runs(&self) -> Result<Vec<ProjectTodoRunRecord>> {
        project_crud_runs::list_running_todo_runs(&self.pool, self.starrocks).await
    }
    async fn next_todo_version(&self, todo_id: &str) -> Result<i64> {
        project_crud_runs::next_todo_version(&self.pool, self.starrocks, todo_id).await
    }
}

// ---- helpers shared by the crud submodules ----

/// One bound value in a dynamically built statement. `TextOrNull` covers both
/// "set to string" and "clear to NULL" (sqlx encodes `None` as NULL).
#[derive(Clone)]
pub(super) enum Arg {
    Text(String),
    TextOrNull(Option<String>),
    Int(i64),
    IntOrNull(Option<i64>),
}

/// Bind `args` in order onto `q` (each `?` in the SQL matches one `Arg`).
pub(super) fn bind_args<'q>(
    mut q: sqlx::query::Query<'q, sqlx::MySql, MySqlArguments>,
    args: Vec<Arg>,
) -> sqlx::query::Query<'q, sqlx::MySql, MySqlArguments> {
    for a in args {
        q = match a {
            Arg::Text(v) => q.bind(v),
            Arg::TextOrNull(v) => q.bind(v),
            Arg::Int(v) => q.bind(v),
            Arg::IntOrNull(v) => q.bind(v),
        };
    }
    q
}

/// One `?` rendered as a literal for the StarRocks text protocol.
fn render_arg(a: &Arg) -> String {
    match a {
        Arg::Text(v) | Arg::TextOrNull(Some(v)) => quote_literal(v),
        Arg::TextOrNull(None) | Arg::IntOrNull(None) => "NULL".to_string(),
        Arg::Int(v) => v.to_string(),
        Arg::IntOrNull(Some(v)) => v.to_string(),
    }
}

/// MySQL-dialect string literal: `''` for quotes, `\\` for backslashes
/// (backslash escapes are enabled by the server default on both MySQL and
/// StarRocks). ids and markdown are the only string data this store writes.
fn quote_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Replace each `?` in `sql` with the rendered literal of the matching arg.
/// The write statements this module builds never contain a literal `?`
/// outside a placeholder.
fn inline_args(sql: &str, args: &[Arg]) -> Result<String> {
    let mut it = args.iter();
    let mut out = String::with_capacity(sql.len() + 16);
    for c in sql.chars() {
        if c == '?' {
            let a = it
                .next()
                .ok_or_else(|| anyhow::anyhow!("placeholder without argument: {sql}"))?;
            out.push_str(&render_arg(a));
        } else {
            out.push(c);
        }
    }
    anyhow::ensure!(it.next().is_none(), "argument without placeholder: {sql}");
    Ok(out)
}

/// Execute one write statement (INSERT/UPDATE/DELETE) and return the number
/// of affected rows. MySQL: the prepared protocol with bound args. StarRocks:
/// its prepared-statement protocol supports SELECT only — INSERT/UPDATE/
/// DELETE fail with ER_UNSUPPORTED_PS (1295) — so writes run over the text
/// protocol with escaped literals inlined.
pub(super) async fn exec_write(
    pool: &MySqlPool,
    starrocks: bool,
    sql: &str,
    args: Vec<Arg>,
) -> Result<u64> {
    let res = if starrocks {
        let rendered = inline_args(sql, &args)?;
        sqlx::raw_sql(&rendered).execute(pool).await
    } else {
        bind_args(sqlx::query(sql), args).execute(pool).await
    };
    let res = res.context("execute project write")?;
    Ok(res.rows_affected())
}

/// Fetch rows for one SELECT. StarRocks runs every statement over the text
/// protocol — not just writes (which the prepared protocol rejects with
/// ER_UNSUPPORTED_PS 1295) but reads too: cached prepared SELECTs were
/// observed returning stale snapshots after same-session raw-protocol
/// writes, so prepared statements are avoided there entirely.
pub(super) async fn exec_read_all(
    pool: &MySqlPool,
    starrocks: bool,
    sql: &str,
    args: &[Arg],
) -> Result<Vec<sqlx::mysql::MySqlRow>> {
    if starrocks {
        let rendered = inline_args(sql, args)?;
        sqlx::raw_sql(&rendered)
            .fetch_all(pool)
            .await
            .context("query project rows")
    } else {
        bind_args(sqlx::query(sql), args.to_vec())
            .fetch_all(pool)
            .await
            .context("query project rows")
    }
}

/// [`exec_read_all`] for `LIMIT 1`-style single-row reads.
pub(super) async fn exec_read_opt(
    pool: &MySqlPool,
    starrocks: bool,
    sql: &str,
    args: &[Arg],
) -> Result<Option<sqlx::mysql::MySqlRow>> {
    if starrocks {
        // sqlx 0.8.6's `RawSql::fetch_optional` is misnamed: it delegates to
        // `fetch_one` and errors with RowNotFound on an empty result, so the
        // optional shape is recovered from `fetch_all`.
        let rendered = inline_args(sql, args)?;
        let rows = sqlx::raw_sql(&rendered)
            .fetch_all(pool)
            .await
            .context("query project row")?;
        Ok(rows.into_iter().next())
    } else {
        Ok(bind_args(sqlx::query(sql), args.to_vec())
            .fetch_optional(pool)
            .await
            .context("query project row")?)
    }
}

/// An unparseable status/kind string is corruption: propagate, never coerce.
pub(super) fn corrupt_status(col: &str, s: &str) -> anyhow::Error {
    anyhow::anyhow!("corrupt project row: unknown status {s} (column {col})")
}

/// `SELECT 1 … WHERE id = ?` existence probe used by the cascade deletes.
pub(super) async fn row_exists(
    pool: &MySqlPool,
    starrocks: bool,
    sql: &str,
    id: &str,
) -> Result<bool> {
    Ok(
        exec_read_opt(pool, starrocks, sql, &[Arg::Text(id.to_string())])
            .await?
            .is_some(),
    )
}

/// Collect one string column (`name`) from already-fetched rows.
pub(super) fn id_column(rows: &[sqlx::mysql::MySqlRow], name: &str) -> Result<Vec<String>> {
    rows.iter()
        .map(|r| r.try_get::<String, _>(name))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read project id column")
}

/// Run a cascade of single-bind DELETEs. MySQL: one transaction, so a
/// mid-cascade failure leaves the tree intact. StarRocks: sequential
/// statements over the text protocol — cross-statement transactions are not
/// guaranteed there, so we lean on the up-front existence check and
/// single-server usage for practical atomicity.
pub(super) async fn run_cascade(
    pool: &MySqlPool,
    starrocks: bool,
    stmts: &[(&'static str, String)],
) -> Result<()> {
    if starrocks {
        for (sql, id) in stmts {
            let rendered = inline_args(sql, &[Arg::Text(id.clone())])?;
            sqlx::raw_sql(&rendered)
                .execute(pool)
                .await
                .with_context(|| format!("starrocks cascade {sql}"))?;
        }
        return Ok(());
    }
    let mut tx = pool.begin().await.context("begin cascade tx")?;
    for (sql, id) in stmts {
        sqlx::query(sql)
            .bind(id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("cascade {sql}"))?;
    }
    tx.commit().await.context("commit cascade tx")
}
