//! Central registry for tmux-owned (`ts`) sessions.
//!
//! Before this module, `opencoder ts -l` scanned **every** per-workdir store
//! under the data root — opening each `opencoder.db`, paging all sessions,
//! reading a `workdir` marker file — then filtered `model IS NULL` in memory.
//! All ts commands now read this single index (`<data_root>/ts.db`) instead;
//! session content stays in the per-workdir stores.
//!
//! The registry uses Turso WAL with independent reader connections, as does
//! `TursoStore`. Rows are the session index: id, durable workdir, owning
//! store dir, timestamps, title and preview. A `meta` table carries the
//! `migrated=1` marker for the one-time legacy import.
//!
//! Ownership: every process may write the file at once. A writer pool joins
//! the shared WAL and holds a shared maintenance lease on
//! `<db>.maintenance.lock` for its lifetime; its transactions serialize with
//! every other writer's (here or in another process) through the engine's
//! WAL writer lock, taken by `BEGIN IMMEDIATE` with the bounded busy retry
//! of `opencoder_database::busy`. A long-running process (the TUI mirror)
//! keeps a reader ([`TsRegistry::open_reader`]) and opens a short-lived
//! writer per mutation through [`TsRegistry::open`]; the CLI takes the
//! writer for the duration of its mutation. Offline maintenance of the file
//! (a TRUNCATE checkpoint) is refused while any writer is alive, through
//! that lease.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use opencoder_database::{busy, Database};

const CREATE_TS_SESSIONS: &str = "\
CREATE TABLE IF NOT EXISTS ts_sessions (
  id         TEXT PRIMARY KEY,
  workdir    TEXT,
  store_dir  TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  title      TEXT,
  preview    TEXT
)";
const CREATE_META: &str = "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)";

/// One indexed ts session. `workdir`/`store_dir` are `None` only for legacy
/// rows whose migration-time marker was missing (`store_dir` is practically
/// always present — it is the store directory the session was found in).
#[derive(Debug, Clone, Default)]
pub struct TsRecord {
    pub id: String,
    pub workdir: Option<PathBuf>,
    pub store_dir: Option<PathBuf>,
    pub created_at: i64,
    pub updated_at: i64,
    pub title: Option<String>,
    pub preview: String,
}

/// Registry over `ts_sessions` + `meta`, using the shared connection pool.
pub struct TsRegistry {
    db: Database,
}

impl TsRegistry {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::open(path).await?;
        db.write(|conn| async move {
            let conn = &conn;
            busy::run_tx(conn, "BEGIN IMMEDIATE", || async move {
                conn.execute_batch(CREATE_TS_SESSIONS).await?;
                conn.execute_batch(CREATE_META).await?;
                Ok(())
            })
            .await
        })
        .await?;
        Ok(Self { db })
    }

    pub async fn open_memory() -> Result<Self> {
        Self::open(":memory:").await
    }

    /// Read-only registry for inspection: no writer lock, no schema install
    /// and no writes; a live TUI owning the registry keeps working beside it.
    pub async fn open_reader(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            db: Database::open_reader(path).await?,
        })
    }

    /// Idempotent insert-or-replace of one session row.
    pub async fn upsert(&self, record: &TsRecord) -> Result<()> {
        let record = record.to_owned();
        self.db
            .write(move |conn| async move {
                let record = &record;
                let conn = &conn;
                busy::run_tx(conn, "BEGIN IMMEDIATE", || async move {
                    conn.execute(
                        "INSERT OR REPLACE INTO ts_sessions \
                 (id, workdir, store_dir, created_at, updated_at, title, preview) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        turso::params![
                            record.id.clone(),
                            record
                                .workdir
                                .as_ref()
                                .map(|p| p.to_string_lossy().into_owned()),
                            record
                                .store_dir
                                .as_ref()
                                .map(|p| p.to_string_lossy().into_owned()),
                            record.created_at,
                            record.updated_at,
                            record.title.clone(),
                            record.preview.clone(),
                        ],
                    )
                    .await
                    .with_context(|| format!("upsert ts session {}", record.id))?;
                    Ok(())
                })
                .await
            })
            .await
    }

    /// All registered sessions, ordered by id for deterministic iteration.
    pub async fn list(&self) -> Result<Vec<TsRecord>> {
        self.db
            .read(move |conn| async move {
                let mut rows = conn
                    .query(
                        "SELECT id, workdir, store_dir, created_at, updated_at, title, preview \
                    FROM ts_sessions ORDER BY id",
                        (),
                    )
                    .await
                    .context("list ts sessions")?;
                let mut out = Vec::new();
                while let Some(row) = rows.next().await? {
                    out.push(TsRecord {
                        id: row.get::<String>(0)?,
                        workdir: row.get::<Option<String>>(1)?.map(PathBuf::from),
                        store_dir: row.get::<Option<String>>(2)?.map(PathBuf::from),
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        title: row.get(5)?,
                        preview: row.get::<Option<String>>(6)?.unwrap_or_default(),
                    });
                }
                Ok(out)
            })
            .await
    }

    /// One session row by id.
    pub async fn get(&self, id: &str) -> Result<Option<TsRecord>> {
        let id = id.to_owned();
        self.db
            .read(move |conn| async move {
                let id = id.as_str();

                let mut rows = conn
                    .query(
                        "SELECT id, workdir, store_dir, created_at, updated_at, title, preview \
                 FROM ts_sessions WHERE id = ?1",
                        turso::params![id.to_string()],
                    )
                    .await
                    .context("get ts session")?;
                let Some(row) = rows.next().await? else {
                    return Ok(None);
                };
                Ok(Some(TsRecord {
                    id: row.get::<String>(0)?,
                    workdir: row.get::<Option<String>>(1)?.map(PathBuf::from),
                    store_dir: row.get::<Option<String>>(2)?.map(PathBuf::from),
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    title: row.get(5)?,
                    preview: row.get::<Option<String>>(6)?.unwrap_or_default(),
                }))
            })
            .await
    }

    /// Remove one session row (no-op when absent).
    pub async fn delete(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.db
            .write(move |conn| async move {
                let id = id.as_str();
                let conn = &conn;
                busy::run_tx(conn, "BEGIN IMMEDIATE", || async move {
                    conn.execute(
                        "DELETE FROM ts_sessions WHERE id = ?1",
                        turso::params![id.to_string()],
                    )
                    .await
                    .with_context(|| format!("delete ts session {id}"))?;
                    Ok(())
                })
                .await
            })
            .await
    }

    /// Whether the one-time legacy store scan has completed.
    pub async fn is_migrated(&self) -> Result<bool> {
        self.db
            .read(move |conn| async move {
                let mut rows = conn
                    .query("SELECT value FROM meta WHERE key = 'migrated'", ())
                    .await
                    .context("read ts registry meta")?;
                let Some(row) = rows.next().await? else {
                    return Ok(false);
                };
                Ok(row.get::<Option<String>>(0)?.as_deref() == Some("1"))
            })
            .await
    }

    /// Mark the legacy migration as complete.
    pub async fn mark_migrated(&self) -> Result<()> {
        self.db
            .write(move |conn| async move {
                let conn = &conn;
                busy::run_tx(conn, "BEGIN IMMEDIATE", || async move {
                    conn.execute(
                        "INSERT OR REPLACE INTO meta (key, value) VALUES ('migrated', '1')",
                        (),
                    )
                    .await
                    .context("mark ts registry migrated")?;
                    Ok(())
                })
                .await
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, workdir: Option<&str>) -> TsRecord {
        TsRecord {
            id: id.to_string(),
            workdir: workdir.map(PathBuf::from),
            store_dir: Some(PathBuf::from("/data/store")),
            created_at: 100,
            updated_at: 200,
            title: Some("task".into()),
            preview: "hello world".into(),
        }
    }

    #[tokio::test]
    async fn upsert_get_list_roundtrip() {
        let registry = TsRegistry::open_memory().await.unwrap();
        registry
            .upsert(&record("01AAA", Some("/work/a")))
            .await
            .unwrap();
        registry.upsert(&record("02BBB", None)).await.unwrap();

        let got = registry.get("01AAA").await.unwrap().expect("row present");
        assert_eq!(got.id, "01AAA");
        assert_eq!(got.workdir.as_deref(), Some(Path::new("/work/a")));
        assert_eq!(got.store_dir.as_deref(), Some(Path::new("/data/store")));
        assert_eq!(got.created_at, 100);
        assert_eq!(got.updated_at, 200);
        assert_eq!(got.title.as_deref(), Some("task"));
        assert_eq!(got.preview, "hello world");

        let list = registry.list().await.unwrap();
        assert_eq!(list.len(), 2, "both rows listed");
        assert_eq!(list[0].id, "01AAA", "ordered by id");
        assert_eq!(list[1].id, "02BBB");

        assert!(registry.get("MISSING").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_replaces() {
        let registry = TsRegistry::open_memory().await.unwrap();
        registry
            .upsert(&record("01AAA", Some("/work/a")))
            .await
            .unwrap();
        let mut replacement = record("01AAA", Some("/work/b"));
        replacement.preview = "updated".into();
        registry.upsert(&replacement).await.unwrap();

        let got = registry.get("01AAA").await.unwrap().unwrap();
        assert_eq!(got.workdir.as_deref(), Some(Path::new("/work/b")));
        assert_eq!(got.preview, "updated");
        assert_eq!(registry.list().await.unwrap().len(), 1, "no duplicate rows");
    }

    #[tokio::test]
    async fn delete_removes_only_the_target() {
        let registry = TsRegistry::open_memory().await.unwrap();
        registry.upsert(&record("01AAA", None)).await.unwrap();
        registry.upsert(&record("02BBB", None)).await.unwrap();
        registry.delete("01AAA").await.unwrap();

        assert!(registry.get("01AAA").await.unwrap().is_none());
        assert!(registry.get("02BBB").await.unwrap().is_some());
        registry.delete("01AAA").await.unwrap(); // no-op, not an error
    }

    #[tokio::test]
    async fn meta_marker_roundtrip() {
        let registry = TsRegistry::open_memory().await.unwrap();
        assert!(!registry.is_migrated().await.unwrap(), "fresh registry");
        registry.mark_migrated().await.unwrap();
        assert!(registry.is_migrated().await.unwrap());
    }

    #[tokio::test]
    async fn on_disk_open_is_reopenable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ts.db");
        {
            let registry = TsRegistry::open(&path).await.unwrap();
            registry
                .upsert(&record("01AAA", Some("/work/a")))
                .await
                .unwrap();
            registry.mark_migrated().await.unwrap();
        }
        let reopened = TsRegistry::open(&path).await.unwrap();
        assert!(reopened.get("01AAA").await.unwrap().is_some());
        assert!(reopened.is_migrated().await.unwrap());
    }

    /// A writer pool leases the registry file for its whole lifetime: offline
    /// maintenance is refused beside it with the lease named as the cause and
    /// admitted once the writer is dropped. A reader takes no lease, so
    /// maintenance proceeds beside it while its own writes stay refused.
    #[tokio::test]
    async fn a_writer_leases_the_registry_and_a_reader_takes_nothing() {
        use opencoder_database::maintenance_lock::{self, is_writers_active, Mode};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ts.db");
        let registry = TsRegistry::open(&path).await.unwrap();
        registry.upsert(&record("01AAA", None)).await.unwrap();
        let key = path.canonicalize().unwrap();
        let refused = maintenance_lock::try_acquire_exclusive(&key).unwrap_err();
        assert!(is_writers_active(&refused), "{refused:#}");
        assert!(
            refused.to_string().contains(&key.display().to_string()),
            "the refusal names the registry: {refused}"
        );
        drop(registry);
        drop(maintenance_lock::try_acquire_exclusive(&key).unwrap());

        let reader = TsRegistry::open_reader(&path).await.unwrap();
        let lease = maintenance_lock::try_acquire_exclusive(&key)
            .expect("a reader holds no lease: maintenance is admitted beside it");
        assert_eq!(lease.mode(), Mode::Exclusive);
        assert_eq!(lease.database(), key.as_path());
        drop(lease);
        assert!(reader.get("01AAA").await.unwrap().is_some());
        let refused = reader.upsert(&record("02BBB", None)).await.unwrap_err();
        assert!(refused.to_string().contains("read-only"), "{refused}");
        assert!(reader.get("02BBB").await.unwrap().is_none());
    }

    /// Writers never exclude each other: a second handle opened beside the
    /// first joins the same WAL, both commit, both read every row and a
    /// reader opened before either of them follows both commits.
    #[tokio::test]
    async fn two_writers_commit_beside_each_other_and_a_reader_follows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ts.db");
        let first = TsRegistry::open(&path).await.unwrap();
        let reader = TsRegistry::open_reader(&path).await.unwrap();
        let second = TsRegistry::open(&path).await.unwrap();
        first.upsert(&record("01AAA", None)).await.unwrap();
        second.upsert(&record("02BBB", None)).await.unwrap();
        assert!(
            first.get("02BBB").await.unwrap().is_some(),
            "the first writer sees the second's commit"
        );
        assert!(
            second.get("01AAA").await.unwrap().is_some(),
            "the second writer sees the first's commit"
        );
        assert_eq!(reader.list().await.unwrap().len(), 2);
        second.delete("01AAA").await.unwrap();
        assert!(first.get("01AAA").await.unwrap().is_none());
        assert!(reader.get("01AAA").await.unwrap().is_none());
        assert_eq!(reader.list().await.unwrap().len(), 1);
    }
}
