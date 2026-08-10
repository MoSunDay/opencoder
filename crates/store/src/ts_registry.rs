//! Central registry for tmux-owned (`ts`) sessions.
//!
//! Before this module, `opencode ts -l` scanned **every** per-workdir store
//! under the data root — opening each `opencoder.db`, paging all sessions,
//! reading a `workdir` marker file — then filtered `model IS NULL` in memory.
//! All ts commands now read this single index (`<data_root>/ts.db`) instead;
//! session content stays in the per-workdir stores.
//!
//! The registry is a small libsql database with the same WAL + serialized
//! access pattern as `LibsqlStore`. Rows are the ts session index: id, durable
//! workdir, owning store dir, timestamps, title and preview. A `meta` table
//! carries the `migrated=1` marker for the one-time legacy import.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use libsql::{Builder, Connection};
use tokio::sync::Mutex;

use crate::libsql_store::schema;

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
const CREATE_META: &str =
    "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)";

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

/// Serialized-access registry over `ts_sessions` + `meta`. Same pragma/WAL
/// setup and `Mutex` discipline as `LibsqlStore`; every method is one
/// statement, so no transaction helper is needed.
pub struct TsRegistry {
    conn: Connection,
    db_lock: Mutex<()>,
}

impl TsRegistry {
    /// Open (or create) the registry database and bootstrap its schema.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Builder::new_local(path.as_ref())
            .build()
            .await
            .with_context(|| format!("open ts registry db at {}", path.as_ref().display()))?;
        let conn = db.connect().context("connect ts registry")?;
        schema::apply_connection_pragmas(&conn).await?;
        let _ = conn.busy_timeout(std::time::Duration::from_secs(30));
        for create in [CREATE_TS_SESSIONS, CREATE_META] {
            conn.execute(create, ())
                .await
                .with_context(|| format!("bootstrap ts registry: {create}"))?;
        }
        Ok(TsRegistry {
            conn,
            db_lock: Mutex::new(()),
        })
    }

    /// Open an in-memory registry (tests and ephemeral callers).
    pub async fn open_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .context("open in-memory ts registry")?;
        let conn = db.connect().context("connect ts registry")?;
        schema::apply_connection_pragmas(&conn).await?;
        for create in [CREATE_TS_SESSIONS, CREATE_META] {
            conn.execute(create, ())
                .await
                .with_context(|| format!("bootstrap ts registry: {create}"))?;
        }
        Ok(TsRegistry {
            conn,
            db_lock: Mutex::new(()),
        })
    }

    /// Idempotent insert-or-replace of one session row.
    pub async fn upsert(&self, record: &TsRecord) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO ts_sessions \
                 (id, workdir, store_dir, created_at, updated_at, title, preview) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                libsql::params![
                    record.id.clone(),
                    record.workdir.as_ref().map(|p| p.to_string_lossy().into_owned()),
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
    }

    /// All registered sessions, ordered by id for deterministic iteration.
    pub async fn list(&self) -> Result<Vec<TsRecord>> {
        let _guard = self.db_lock.lock().await;
        let mut rows = self
            .conn
            .query("SELECT id, workdir, store_dir, created_at, updated_at, title, preview \
                    FROM ts_sessions ORDER BY id",
                (),
            )
            .await
            .context("list ts sessions")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(TsRecord {
                id: row.get::<String>(0)?,
                workdir: row
                    .get::<Option<String>>(1)?
                    .map(PathBuf::from),
                store_dir: row
                    .get::<Option<String>>(2)?
                    .map(PathBuf::from),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                title: row.get(5)?,
                preview: row.get::<Option<String>>(6)?.unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// One session row by id.
    pub async fn get(&self, id: &str) -> Result<Option<TsRecord>> {
        let _guard = self.db_lock.lock().await;
        let mut rows = self
            .conn
            .query(
                "SELECT id, workdir, store_dir, created_at, updated_at, title, preview \
                 FROM ts_sessions WHERE id = ?1",
                libsql::params![id.to_string()],
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
    }

    /// Remove one session row (no-op when absent).
    pub async fn delete(&self, id: &str) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        self.conn
            .execute(
                "DELETE FROM ts_sessions WHERE id = ?1",
                libsql::params![id.to_string()],
            )
            .await
            .with_context(|| format!("delete ts session {id}"))?;
        Ok(())
    }

    /// Whether the one-time legacy store scan has completed.
    pub async fn is_migrated(&self) -> Result<bool> {
        let _guard = self.db_lock.lock().await;
        let mut rows = self
            .conn
            .query("SELECT value FROM meta WHERE key = 'migrated'", ())
            .await
            .context("read ts registry meta")?;
        let Some(row) = rows.next().await? else {
            return Ok(false);
        };
        Ok(row.get::<Option<String>>(0)?.as_deref() == Some("1"))
    }

    /// Mark the legacy migration as complete.
    pub async fn mark_migrated(&self) -> Result<()> {
        let _guard = self.db_lock.lock().await;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('migrated', '1')",
                (),
            )
            .await
            .context("mark ts registry migrated")?;
        Ok(())
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
        registry.upsert(&record("01AAA", Some("/work/a"))).await.unwrap();
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
        registry.upsert(&record("01AAA", Some("/work/a"))).await.unwrap();
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
            registry.upsert(&record("01AAA", Some("/work/a"))).await.unwrap();
            registry.mark_migrated().await.unwrap();
        }
        let reopened = TsRegistry::open(&path).await.unwrap();
        assert!(reopened.get("01AAA").await.unwrap().is_some());
        assert!(reopened.is_migrated().await.unwrap());
    }
}
