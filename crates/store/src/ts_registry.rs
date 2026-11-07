//! Central registry for tmux-owned (`ts`) sessions.
//!
//! All ts commands read this single index (`<data_root>/ts.db.rdb`, see
//! [`TsRegistry::db_dir`]) instead of scanning every per-workdir store;
//! session content stays in the per-workdir stores. Rows are the session
//! index: id, durable workdir, owning store dir, timestamps, title and
//! preview. A `meta/migrated` marker carries the one-time legacy import
//! flag.
//!
//! RocksDB needs a directory, so the legacy file name is preserved as the
//! directory stem: `ts.db` → `ts.db.rdb`. The legacy stem was always a plain
//! SQLite *file*, so — unlike the per-workdir session/runtime stores — there
//! is no development-layout directory variant to fall back to.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::rdb_store::engine::RdbDb;
use crate::rdb_store::{kv, tx};

const ROW_PREFIX: &[u8] = b"ts/";
const META_KEY: &[u8] = b"meta/migrated";

/// One indexed ts session. `workdir`/`store_dir` are `None` only for legacy
/// rows whose migration-time marker was missing (`store_dir` is practically
/// always present — it is the store directory the session was found in).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TsRecord {
    pub id: String,
    #[serde(default)]
    pub workdir: Option<PathBuf>,
    #[serde(default)]
    pub store_dir: Option<PathBuf>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub preview: String,
}

impl TsRecord {
    fn key(id: &str) -> Vec<u8> {
        let mut key = ROW_PREFIX.to_vec();
        key.extend_from_slice(id.as_bytes());
        key
    }
}

/// Registry over `ts/<id>` rows + the migrated marker, on the rdb engine.
pub struct TsRegistry {
    db: RdbDb,
    /// Keeps an `open_memory()` temp directory alive for the registry's life.
    _tmp: Option<Arc<tempfile::TempDir>>,
}

impl TsRegistry {
    /// Pure path policy: the registry's RocksDB directory for a legacy
    /// `ts.db` stem — `<legacy>.rdb` (e.g. `<data_root>/ts.db.rdb`). Pure so
    /// callers can detect an existing registry without opening (and thereby
    /// locking or creating) it.
    pub fn db_dir(legacy_ts_db: &Path) -> PathBuf {
        PathBuf::from(format!("{}.rdb", legacy_ts_db.display()))
    }

    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let dir = Self::db_dir(path.as_ref());
        Ok(Self {
            db: RdbDb::open(&dir)?,
            _tmp: None,
        })
    }

    pub async fn open_memory() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let db = RdbDb::open(dir.path())?;
        Ok(Self {
            db,
            _tmp: Some(Arc::new(dir)),
        })
    }

    /// Idempotent insert-or-replace of one session row.
    pub async fn upsert(&self, record: &TsRecord) -> Result<()> {
        let key = TsRecord::key(&record.id);
        let payload = kv::encode(record)?;
        self.db
            .write(move |store| {
                let mut batch = rocksdb::WriteBatch::default();
                tx::put_raw(&mut batch, &key, &payload);
                tx::commit(store, batch)
            })
            .await
            .with_context(|| format!("upsert ts session {}", record.id))
    }

    /// All registered sessions, ordered by id for deterministic iteration.
    pub async fn list(&self) -> Result<Vec<TsRecord>> {
        self.db
            .read(|store| {
                let mut out = Vec::new();
                for (_, value) in tx::scan_prefix(store, ROW_PREFIX)? {
                    out.push(kv::decode(&value)?);
                }
                Ok(out)
            })
            .await
            .context("list ts sessions")
    }

    pub async fn get(&self, id: &str) -> Result<Option<TsRecord>> {
        let id = id.to_owned();
        self.db
            .read(move |store| kv::get_json(store, &TsRecord::key(&id)))
            .await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let key = TsRecord::key(id);
        self.db
            .write(move |store| {
                let mut batch = rocksdb::WriteBatch::default();
                tx::del(&mut batch, &key);
                tx::commit(store, batch)
            })
            .await
            .with_context(|| format!("delete ts session {id}"))
    }

    pub async fn is_migrated(&self) -> Result<bool> {
        self.db
            .read(|store| {
                Ok(rdb::store::get(store, b"", META_KEY)
                    .map_err(kv::engine_err)?
                    .is_some())
            })
            .await
            .context("read ts registry meta")
    }

    pub async fn mark_migrated(&self) -> Result<()> {
        self.db
            .write(|store| {
                let mut batch = rocksdb::WriteBatch::default();
                tx::put_raw(&mut batch, META_KEY, b"1");
                tx::commit(store, batch)
            })
            .await
            .context("mark ts registry migrated")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, workdir: Option<&str>) -> TsRecord {
        TsRecord {
            id: id.into(),
            workdir: workdir.map(PathBuf::from),
            store_dir: Some(PathBuf::from("/store")),
            created_at: 1,
            updated_at: 2,
            title: Some("title".into()),
            preview: "preview".into(),
        }
    }

    #[tokio::test]
    async fn upsert_get_list_roundtrip() {
        let registry = TsRegistry::open_memory().await.unwrap();
        registry.upsert(&record("a", Some("/a"))).await.unwrap();
        registry.upsert(&record("b", None)).await.unwrap();
        let listed = registry.list().await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "a", "ordered by id");
        assert_eq!(listed[1].id, "b");
        assert_eq!(listed[1].workdir, None);
        let got = registry.get("a").await.unwrap().unwrap();
        assert_eq!(got.workdir.as_deref(), Some(Path::new("/a")));
        assert!(registry.get("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_replaces() {
        let registry = TsRegistry::open_memory().await.unwrap();
        registry.upsert(&record("a", Some("/a"))).await.unwrap();
        let mut updated = record("a", Some("/a2"));
        updated.title = Some("new".into());
        registry.upsert(&updated).await.unwrap();
        let listed = registry.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].workdir.as_deref(), Some(Path::new("/a2")));
        assert_eq!(listed[0].title.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn delete_removes_only_the_target() {
        let registry = TsRegistry::open_memory().await.unwrap();
        registry.upsert(&record("a", Some("/a"))).await.unwrap();
        registry.upsert(&record("b", Some("/b"))).await.unwrap();
        registry.delete("a").await.unwrap();
        let listed = registry.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "b");
        assert!(registry.get("a").await.unwrap().is_none());
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
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ts.db");
        {
            let registry = TsRegistry::open(&path).await.unwrap();
            registry.upsert(&record("a", Some("/a"))).await.unwrap();
            registry.mark_migrated().await.unwrap();
        }
        let reopened = TsRegistry::open(&path).await.unwrap();
        assert!(reopened.is_migrated().await.unwrap());
        assert_eq!(reopened.list().await.unwrap().len(), 1);
    }

    /// The pure path helper names the on-disk directory `open` really uses.
    #[test]
    fn db_dir_appends_rdb_suffix() {
        let dir = TsRegistry::db_dir(Path::new("/data/opencoder/ts.db"));
        assert_eq!(dir, Path::new("/data/opencoder/ts.db.rdb"));
    }
}
