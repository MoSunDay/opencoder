/// Transaction body of [`bootstrap`]; runs with the write lock already held.
///
/// Statement order is load-bearing: the pre-migration DDL batch, then the
/// version check / migrate / set_version branch, then the post-migration
/// index batch (some indexes target columns that only exist after `migrate`).
async fn bootstrap_tx(conn: &Connection) -> Result<()> {
    // Probe BEFORE the DDL batch: afterwards every table exists (pre-existing
    // ones via the `IF NOT EXISTS` no-ops, missing ones freshly created), so
    // this is the only point where a legacy database - tables written before
    // schema_version tracking, version row absent - is distinguishable from a
    // fresh one. `sessions` is the oldest app table and therefore a
    // sufficient marker of a pre-tracking database.
    let preexisting = table_exists(conn, "sessions").await?;
    conn.execute(CREATE_SCHEMA_VERSION, ()).await?;
    let current = current_version(conn).await?;
    anyhow::ensure!(
        current.is_none_or(|version| version <= SCHEMA_VERSION),
        "database schema is newer than supported version {SCHEMA_VERSION}; use a compatible release"
    );
    conn.execute(CREATE_SESSIONS, ()).await?;
    conn.execute(CREATE_MESSAGES, ()).await?;
    conn.execute(CREATE_INPUTS, ()).await?;
    conn.execute(CREATE_EVENTS, ()).await?;
    conn.execute(CREATE_SUBAGENT_TASKS, ()).await?;
    conn.execute(CREATE_TODO_WORKFLOWS, ()).await?;
    conn.execute(CREATE_TODO_ITEMS, ()).await?;
    conn.execute(CREATE_TODO_EVENTS, ()).await?;
    conn.execute(CREATE_NODES, ()).await?;
    conn.execute(CREATE_NODE_TASKS, ()).await?;
    conn.execute(CREATE_BRAIN_CAPABILITIES, ()).await?;
    conn.execute(CREATE_BRAIN_ENG_INPUTS, ()).await?;
    conn.execute(CREATE_BRAIN_VECTORS, ()).await?;
    super::brain_layered::initialize(conn).await?;
    conn.execute(CREATE_PROJECT_GOALS, ()).await?;
    if current.map_or(preexisting, |version| version < 32) {
        conn.execute(CREATE_PROJECT_MILESTONES, ()).await?;
        conn.execute(
            &CREATE_PROJECT_TODOS.replace("initiative_id", "milestone_id"),
            (),
        )
        .await?;
    }
    conn.execute(CREATE_PROJECT_INITIATIVES, ()).await?;
    catalog::initialize_tags(conn).await?;
    conn.execute(CREATE_PROJECT_TODOS, ()).await?;
    conn.execute(CREATE_PROJECT_TODO_RUNS, ()).await?;
    conn.execute(CREATE_PROJECT_TODO_EXECUTIONS, ()).await?;
    conn.execute(CREATE_DAG_DEFS, ()).await?;
    conn.execute(CREATE_DAG_RUNS, ()).await?;
    conn.execute(CREATE_DAG_EVENTS, ()).await?;
    conn.execute(CREATE_TEAM_TOPIC_RUNS, ()).await?;
    conn.execute(CREATE_SCHEDULE_RUNS, ()).await?;
    conn.execute(CREATE_SCHEDULES, ()).await?;
    conn.execute(CREATE_PLATFORM_USERS, ()).await?;
    access::initialize(conn).await?;
    conn.execute(CREATE_INDEX_MSG, ()).await?;
    conn.execute(CREATE_INDEX_IN, ()).await?;
    conn.execute(CREATE_INDEX_EV, ()).await?;
    conn.execute(CREATE_INDEX_SA_PARENT, ()).await?;
    conn.execute(CREATE_INDEX_SA_CHILD, ()).await?;
    conn.execute(CREATE_INDEX_SA_TASK_ID, ()).await?;

    // Incremental migrations: only run when upgrading from a prior version.
    // Fresh databases (version None) already have the full schema from the
    // CREATE TABLE statements above, so migrations are skipped for them.
    if let Some(prev) = current {
        if prev < SCHEMA_VERSION {
            migrate(conn, prev).await?;
            write_version(conn, SCHEMA_VERSION).await?;
        }
    } else if preexisting {
        // Legacy shape, version row absent. `migrate` from v0 is the correct
        // (and simplest) entry: no version row survives to say which partial
        // upgrades ran, and a full pass is safe on ANY historical shape
        // because every step is either `CREATE ... IF NOT EXISTS`, an
        // `add_column_if_absent` guarded by `PRAGMA table_info`, or a
        // conditional backfill UPDATE converging to a fixed point (see
        // `migrate`). Skipping migrate here would stamp the current version
        // over stale tables; the post-migration index batch would then fail
        // on the missing `sessions.task_type`, and - that failure sharing the
        // bootstrap transaction - every later open would roll back and fail
        // again, leaving the database permanently unopenable.
        migrate(conn, 0).await?;
        write_version(conn, SCHEMA_VERSION).await?;
    } else {
        write_version(conn, SCHEMA_VERSION).await?;
    }
    // Historical v32 catalogs can retain milestone columns despite their
    // watermark. Converge the actual shape before creating current indexes.
    catalog::upgrade(conn).await?;
    project_people::initialize(conn).await?;
    super::project::summaries::initialize(conn).await?;
    super::project::comments::initialize(conn).await?;
    // The task_type index depends on a column that only physically exists in
    // fresh databases (via CREATE TABLE) or after the v5 migration adds it for
    // older databases, so it must run AFTER `migrate` rather than in the
    // pre-migration index batch above.
    conn.execute(CREATE_INDEX_SESSION_TASK_TYPE, ()).await?;
    conn.execute(CREATE_INDEX_SESSION_ACTIVITY, ()).await?;
    conn.execute(CREATE_INDEX_TODO_STATUS, ()).await?;
    conn.execute(CREATE_INDEX_TODO_EVENTS, ()).await?;
    // The node_tasks index targets tables that only exist on fresh databases
    // (via the CREATE batch above) or after the v12 migration creates them,
    // so — like the task_type index — it must run AFTER `migrate`.
    conn.execute(CREATE_INDEX_NODE_TASKS_STATUS, ()).await?;
    // Same post-migrate placement: brain tables physically exist either via
    // the CREATE batch above (fresh DBs) or the v15 migration (old DBs).
    conn.execute(CREATE_INDEX_BRAIN_ENG_INPUTS, ()).await?;
    conn.execute(CREATE_INDEX_PROJECT_INITIATIVES_GOAL, ())
        .await?;
    conn.execute(CREATE_INDEX_PROJECT_TODOS_INITIATIVE, ())
        .await?;
    conn.execute(CREATE_INDEX_PROJECT_TODO_RUNS_TODO, ())
        .await?;
    // Same post-migrate placement as the indexes above: the dag tables
    // physically exist either via the CREATE batch (fresh DBs) or the v16
    // migration (old DBs).
    conn.execute(CREATE_INDEX_DAG_RUNS_STATUS_NODE, ()).await?;
    conn.execute(CREATE_INDEX_DAG_EVENTS_RUN_SEQ, ()).await?;
    // Same post-migrate placement: the team ledger physically exists either
    // via the CREATE batch (fresh DBs) or the v17 migration (old DBs).
    conn.execute(CREATE_INDEX_TEAM_TOPIC_RUNS_TOPIC, ()).await?;
    // Same post-migrate placement: the schedule ledger physically exists
    // either via the CREATE batch (fresh DBs) or the v26 migration (old DBs).
    conn.execute(CREATE_INDEX_SCHEDULE_RUNS_FIRED, ()).await?;
    // Same post-migrate placement: brain plans physically exist either via
    // the CREATE batch (fresh DBs) or the v18 migration (old DBs).
    conn.execute(CREATE_INDEX_IN_ID, ()).await?;
    Ok(())
}

/// Run incremental schema migrations from `from` up to the current version.
///
/// Migrations are idempotent: each `ALTER TABLE ... ADD COLUMN` is guarded by
/// `add_column_if_absent`, which inspects `PRAGMA table_info` and skips the
/// column when it is already present. This is important because `bootstrap`
/// always runs the `CREATE TABLE` statements carrying the *full latest* schema,
/// so a table can already physically carry a column even when `schema_version`
/// records a stale older version (e.g. a database whose schema_version row was
/// left behind at 1 while the tables were recreated at the latest shape). A
/// bare `ADD COLUMN` would fail with `duplicate column name` in that case;
/// guarding the ALTER makes re-migration safe regardless of the
/// CREATE-TABLE-vs-stale-version disagreement.
///
/// `bootstrap_tx` also enters here with `from = 0` for legacy databases whose
/// tables predate schema_version tracking (version row absent): with no row
/// to say which partial upgrades ran, the full pass from the bottom is the
/// only correct entry, and it is safe for exactly the reasons above.
/// Return `true` if `table` has a column named `column`.
///
/// Inspects `PRAGMA table_info(<table>)`, where the column-name lives at result
/// index 1. `table` and `column` are code-controlled literals, so interpolating
/// them into the SQL is safe.
async fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare_cached(&format!("PRAGMA table_info({table})"))
        .await?;
    let mut rows = stmt.query(()).await?;
    while let Some(row) = rows.next().await? {
        let name: String = row.get::<String>(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Return `true` if a table with this name exists in the database.
///
/// Reads `sqlite_master` rather than `PRAGMA table_info` - the latter returns
/// zero rows both for "table without columns" and "no table", and only the
/// second case matters here. The name is bound as a parameter (no SQL
/// interpolation), so the caller-controlled literal stays safe regardless.
async fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare_cached("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1")
        .await?;
    let mut rows = stmt.query(turso::params![table]).await?;
    Ok(rows.next().await?.is_some())
}

/// `ALTER TABLE {table} ADD COLUMN {column} {type_def}`, but a no-op when the
/// column already exists. `table`, `column`, and `type_def` are code-controlled
/// literals (not user input), so the format! interpolation is intentional.
async fn add_column_if_absent(
    conn: &Connection,
    table: &str,
    column: &str,
    type_def: &str,
) -> Result<()> {
    if column_exists(conn, table, column).await? {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {type_def}"),
        (),
    )
    .await
    .with_context(|| format!("add column {table}.{column}"))?;
    Ok(())
}

pub async fn current_version(conn: &Connection) -> Result<Option<i64>> {
    let mut stmt = conn
        .prepare_cached("SELECT version FROM schema_version LIMIT 1")
        .await?;
    let mut rows = stmt.query(()).await?;
    if let Some(row) = rows.next().await? {
        Ok(Some(row.get::<i64>(0)?))
    } else {
        Ok(None)
    }
}

/// Replace the version row in its own transaction (the standalone write path).
///
/// Kept for callers/tests that need the transactional write in isolation; the
/// bootstrap path inlines the body via `write_version` because it already
/// holds the bootstrap transaction.
#[cfg_attr(not(test), allow(dead_code))]
async fn set_version(conn: &Connection, version: i64) -> Result<()> {
    // Wrap DELETE + INSERT in a single transaction so a crash between them
    // cannot leave schema_version empty (which would trigger a spurious full
    // re-migration on the next boot).
    super::tx::run_tx(conn, "BEGIN", || write_version(conn, version)).await
}

/// DELETE + INSERT the version row WITHOUT a surrounding transaction.
///
/// Standalone transaction body shared by `set_version` (which wraps it in its
/// own `BEGIN`) and `bootstrap_tx` (which already runs inside the bootstrap
/// transaction — nesting another BEGIN there would fail with "cannot start a
/// transaction within a transaction").
async fn write_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute("DELETE FROM schema_version", ()).await?;
    conn.execute(
        "INSERT INTO schema_version(version) VALUES (?1)",
        turso::params![version],
    )
    .await?;
    Ok(())
}
