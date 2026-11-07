//! Durable global registry for tmux-owned sessions.
//!
//! All ts commands read one central index, `<data_root>/ts.db.rdb`
//! (`TsRegistry`), instead of scanning every per-workdir store. The first
//! open performs a one-time bootstrap: every rdb store under the data root
//! is scanned and its `model IS NULL` sessions (the durable ts marker rows
//! written before the central registry existed) are imported. The import is
//! idempotent (an idempotent row upsert plus a trailing `migrated=1` meta
//! marker), so a crash mid-way simply restarts on the next command. Only
//! RocksDB store directories are scanned; plain files are never opened.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use opencoder_store::{RdbStore, SessionFilter, SessionListItem, Store, TsRecord, TsRegistry};

const STORE_PAGE_SIZE: u32 = 500;

/// Open the central registry, running the one-time bootstrap import when the
/// `migrated` meta marker is absent.
pub(crate) async fn open_registry() -> Result<TsRegistry> {
    let root = opencoder_core::data_root();
    let path = root.join("ts.db");
    let registry = TsRegistry::open(&path).await?;
    if !registry.is_migrated().await? {
        import_pre_registry_markers(&registry, &root).await?;
        registry.mark_migrated().await?;
    }
    Ok(registry)
}

/// Register (or refresh) one ts session: canonical workdir, its owning store
/// dir, and a creation timestamp. Replaces the old per-store `workdir` marker
/// file — the registry row is now the durable record.
pub(crate) async fn register(registry: &TsRegistry, id: &str, workdir: &Path) -> Result<()> {
    let canonical = tokio::fs::canonicalize(workdir)
        .await
        .with_context(|| format!("resolve tmux workdir: {}", workdir.display()))?;
    let now = opencoder_core::message::now_ms();
    registry
        .upsert(&TsRecord {
            id: id.to_string(),
            workdir: Some(canonical.clone()),
            store_dir: Some(opencoder_core::data_dir_for(&canonical)),
            created_at: now,
            updated_at: now,
            title: None,
            preview: String::new(),
        })
        .await
}

/// One-time import of per-store ts sessions. Idempotent: re-running after a
/// crash re-imports the same rows and later upserts converge duplicate ids.
async fn import_pre_registry_markers(registry: &TsRegistry, root: &Path) -> Result<()> {
    for stored in scan(root, true).await? {
        if stored.item.model.is_some() {
            continue; // plain tui/run session — never ts-owned
        }
        registry
            .upsert(&TsRecord {
                id: stored.item.id,
                workdir: None,
                store_dir: Some(stored.store_dir),
                created_at: stored.item.created_at,
                updated_at: stored.item.updated_at,
                title: stored.item.title,
                preview: stored.item.preview,
            })
            .await?;
    }
    Ok(())
}

/// Scan every per-workdir store under `root`. A child counts as a store
/// when the canonical session-store path policy resolves to a RocksDB
/// *directory* (`opencoder.db.rdb`). A plain `opencoder.db` file is never
/// opened as a store (RocksDB needs a directory); such children are skipped —
/// this migration step does
/// not import SQLite content. `strict` controls whether an unreadable store
/// aborts the scan (migration wants strict; the old display path tolerated
/// bad stores).
async fn scan(root: &Path, strict: bool) -> Result<Vec<StoredSession>> {
    let mut rd = match tokio::fs::read_dir(root).await {
        Ok(rd) => rd,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("read data root: {}", root.display()));
        }
    };
    let mut out = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        let store_dir = entry.path();
        if !store_dir.is_dir() {
            continue;
        }
        let db = opencoder_core::session_store_rdb_dir(&store_dir);
        if !db.is_dir() {
            continue;
        }
        let loaded = load_store(&store_dir, &db).await;
        match loaded {
            Ok(items) => out.extend(items),
            Err(error) if !strict => {
                tracing::warn!(path = %db.display(), %error, "ts: skipping unreadable store");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(out)
}

async fn load_store(store_dir: &Path, db: &Path) -> Result<Vec<StoredSession>> {
    let store = RdbStore::open(db)
        .await
        .with_context(|| format!("open session store: {}", db.display()))?;
    Ok(list_all_sessions(&store)
        .await
        .with_context(|| format!("list sessions: {}", db.display()))?
        .into_iter()
        .map(|item| StoredSession {
            store_dir: store_dir.to_path_buf(),
            item,
        })
        .collect())
}

async fn list_all_sessions(store: &RdbStore) -> Result<Vec<SessionListItem>> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .list_sessions(&SessionFilter {
                limit: STORE_PAGE_SIZE,
                cursor: cursor.clone(),
                ..Default::default()
            })
            .await?;
        let page_len = page.len();
        cursor = page
            .last()
            .map(|item| format!("{}|{}", item.updated_at.max(item.created_at), item.id));
        out.extend(page);
        if page_len < STORE_PAGE_SIZE as usize {
            return Ok(out);
        }
    }
}

#[derive(Debug, Clone)]
struct StoredSession {
    store_dir: PathBuf,
    item: SessionListItem,
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::SessionMeta;

    fn meta(id: &str, model: Option<&str>, created: i64) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            title: None,
            agent: None,
            model: model.map(String::from),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: created,
            updated_at: created,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            kind: None,
        }
    }

    #[tokio::test]
    async fn register_roundtrip_then_delete() {
        let registry = TsRegistry::open_memory().await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path().join("project");
        tokio::fs::create_dir_all(&workdir).await.unwrap();

        register(&registry, "01AAA", &workdir).await.unwrap();

        let record = registry.get("01AAA").await.unwrap().expect("registered");
        assert_eq!(record.id, "01AAA");
        assert_eq!(
            record.workdir.as_deref(),
            Some(workdir.as_path()),
            "canonical workdir stored"
        );
        assert_eq!(
            record.store_dir.as_deref(),
            Some(opencoder_core::data_dir_for(&workdir).as_path()),
            "owning store dir derived from the canonical workdir"
        );
        assert!(record.title.is_none());
        assert!(record.preview.is_empty());
        assert!(record.created_at > 1_000_000_000_000, "epoch ms");

        registry.delete("01AAA").await.unwrap();
        assert!(registry.get("01AAA").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn migration_imports_ts_owned_sessions_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Store A: ts-owned session (model NULL).
        let h1 = root.join("aaaa");
        tokio::fs::create_dir_all(&h1).await.unwrap();
        {
            let store = RdbStore::open(opencoder_core::session_store_rdb_dir(&h1))
                .await
                .unwrap();
            store.create_session(&meta("TSA1", None, 1)).await.unwrap();
            store
                .create_session(&meta("PLAIN1", Some("m"), 2))
                .await
                .unwrap();
        }
        // Store B: another ts-owned session.
        let h2 = root.join("bbbb");
        tokio::fs::create_dir_all(&h2).await.unwrap();
        {
            let store = RdbStore::open(opencoder_core::session_store_rdb_dir(&h2))
                .await
                .unwrap();
            store.create_session(&meta("TSB1", None, 3)).await.unwrap();
        }

        // A directory without a store, a plain file, and a directory whose
        // only store candidate is a plain FILE: all skipped — a plain file
        // is never opened as an rdb store.
        tokio::fs::create_dir_all(root.join("cccc")).await.unwrap();
        tokio::fs::write(root.join("not-a-store"), "x")
            .await
            .unwrap();
        let plain = root.join("dddd");
        tokio::fs::create_dir_all(&plain).await.unwrap();
        tokio::fs::write(plain.join("opencoder.db"), b"plain sqlite bytes")
            .await
            .unwrap();

        let registry = TsRegistry::open_memory().await.unwrap();
        import_pre_registry_markers(&registry, root).await.unwrap();

        let rows = registry.list().await.unwrap();
        assert_eq!(rows.len(), 2, "TSA1 + TSB1 imported; PLAIN1 skipped");
        let a = registry.get("TSA1").await.unwrap().expect("ts session");
        assert_eq!(a.workdir, None, "scan imports carry no workdir");
        assert_eq!(a.store_dir.as_deref(), Some(h1.as_path()));
        let b = registry.get("TSB1").await.unwrap().expect("ts session");
        assert_eq!(b.workdir, None);
        assert_eq!(b.store_dir.as_deref(), Some(h2.as_path()));
        assert!(registry.get("PLAIN1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn bootstrap_import_is_idempotent_and_crash_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let h1 = tmp.path().join("aaaa");
        tokio::fs::create_dir_all(&h1).await.unwrap();
        let store = RdbStore::open(opencoder_core::session_store_rdb_dir(&h1))
            .await
            .unwrap();
        store.create_session(&meta("TSA1", None, 1)).await.unwrap();
        drop(store);

        let registry = TsRegistry::open_memory().await.unwrap();
        import_pre_registry_markers(&registry, tmp.path())
            .await
            .unwrap();
        // A crash between scan and mark_migrated re-runs the whole import;
        // the idempotent row upsert keeps it duplicate-free.
        import_pre_registry_markers(&registry, tmp.path())
            .await
            .unwrap();
        assert_eq!(registry.list().await.unwrap().len(), 1, "no duplicates");
        registry.mark_migrated().await.unwrap();
        import_pre_registry_markers(&registry, tmp.path())
            .await
            .unwrap();
        assert_eq!(registry.list().await.unwrap().len(), 1);
    }

    /// A store dir holding only a plain `opencoder.db` file is distinguished
    /// from rdb store directories and never opened — the file must survive
    /// the scan byte-for-byte.
    #[tokio::test]
    async fn scan_never_opens_a_plain_file_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let h1 = tmp.path().join("aaaa");
        tokio::fs::create_dir_all(&h1).await.unwrap();
        tokio::fs::write(h1.join("opencoder.db"), b"plain sqlite bytes")
            .await
            .unwrap();

        let scanned = scan(tmp.path(), true).await.unwrap();
        assert!(scanned.is_empty(), "a plain file is not a store");
        let bytes = tokio::fs::read(h1.join("opencoder.db")).await.unwrap();
        assert_eq!(bytes, b"plain sqlite bytes", "file untouched");
        assert!(
            !h1.join("opencoder.db.rdb").exists(),
            "scan must not create an rdb store beside the plain file"
        );
    }

    #[tokio::test]
    async fn list_all_sessions_paginates_past_store_limit() {
        let store = RdbStore::open_memory().await.unwrap();
        for n in 0..=STORE_PAGE_SIZE {
            let mut session = meta(&format!("PAGE{n:04}"), None, i64::from(n));
            session.updated_at = 10_000;
            store.create_session(&session).await.unwrap();
        }
        assert_eq!(
            list_all_sessions(&store).await.unwrap().len(),
            STORE_PAGE_SIZE as usize + 1
        );
    }
}
