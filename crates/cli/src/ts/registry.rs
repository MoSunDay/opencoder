//! Durable global registry for tmux-owned sessions.
//!
//! Each per-workdir store directory carries a small `workdir` marker. This
//! keeps the existing one-store-per-workdir layout while making the hash
//! directory reversible for global list/resume/cleanup operations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use opencoder_store::{LibsqlStore, SessionFilter, SessionListItem, Store};

const STORE_PAGE_SIZE: u32 = 500;
const WORKDIR_MARKER: &str = "workdir";

#[derive(Debug, Clone)]
pub(crate) struct StoredSession {
    pub store_dir: PathBuf,
    pub workdir: Option<PathBuf>,
    pub item: SessionListItem,
}

/// Persist the canonical workdir beside its store and return the store dir.
pub(crate) async fn record_workdir(workdir: &Path) -> Result<PathBuf> {
    let canonical = tokio::fs::canonicalize(workdir)
        .await
        .with_context(|| format!("resolve tmux workdir: {}", workdir.display()))?;
    let store_dir = opencoder_core::data_dir_for(&canonical);
    tokio::fs::create_dir_all(&store_dir)
        .await
        .with_context(|| format!("create session store dir: {}", store_dir.display()))?;
    write_marker(&store_dir, &canonical).await?;
    Ok(store_dir)
}

async fn write_marker(store_dir: &Path, canonical: &Path) -> Result<()> {
    let marker = store_dir.join(WORKDIR_MARKER);
    let temporary = store_dir.join(format!(".{WORKDIR_MARKER}.{}.tmp", std::process::id()));
    tokio::fs::write(&temporary, canonical.to_string_lossy().as_bytes())
        .await
        .with_context(|| format!("write workdir marker: {}", temporary.display()))?;
    tokio::fs::rename(&temporary, &marker)
        .await
        .with_context(|| format!("publish workdir marker: {}", marker.display()))?;
    Ok(())
}

pub(crate) async fn scan_best_effort(root: &Path) -> Vec<StoredSession> {
    match scan(root, false).await {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(path = %root.display(), %error, "ts: cannot scan global registry");
            Vec::new()
        }
    }
}

pub(crate) async fn scan_required(root: &Path) -> Result<Vec<StoredSession>> {
    scan(root, true).await
}

async fn scan(root: &Path, strict: bool) -> Result<Vec<StoredSession>> {
    let mut rd = match tokio::fs::read_dir(root).await {
        Ok(rd) => rd,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("read data root: {}", root.display()))
        }
    };
    let mut out = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        let store_dir = entry.path();
        let db = store_dir.join("opencoder.db");
        if !store_dir.is_dir() || !db.is_file() {
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
    let store = LibsqlStore::open(db)
        .await
        .with_context(|| format!("open session store: {}", db.display()))?;
    let workdir = read_workdir(store_dir).await;
    Ok(list_all_sessions(&store)
        .await
        .with_context(|| format!("list sessions: {}", db.display()))?
        .into_iter()
        .map(|item| StoredSession {
            store_dir: store_dir.to_path_buf(),
            workdir: workdir.clone(),
            item,
        })
        .collect())
}

async fn read_workdir(store_dir: &Path) -> Option<PathBuf> {
    let bytes = tokio::fs::read(store_dir.join(WORKDIR_MARKER)).await.ok()?;
    let path = PathBuf::from(String::from_utf8(bytes).ok()?);
    path.is_absolute().then_some(path)
}

async fn list_all_sessions(store: &LibsqlStore) -> Result<Vec<SessionListItem>> {
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
            .map(|item| format!("{}|{}", item.created_at, item.id));
        out.extend(page);
        if page_len < STORE_PAGE_SIZE as usize {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::SessionMeta;

    #[tokio::test]
    async fn marker_roundtrip_preserves_absolute_workdir() {
        let root = tempfile::tempdir().unwrap();
        let workdir = root.path().join("project");
        tokio::fs::create_dir_all(&workdir).await.unwrap();
        write_marker(root.path(), &workdir).await.unwrap();
        assert_eq!(
            read_workdir(root.path()).await.as_deref(),
            Some(workdir.as_path())
        );
    }

    #[tokio::test]
    async fn list_all_sessions_paginates_past_store_limit() {
        let store = LibsqlStore::open_memory().await.unwrap();
        for n in 0..=STORE_PAGE_SIZE {
            store
                .create_session(&SessionMeta {
                    id: format!("PAGE{n:04}"),
                    title: None,
                    agent: None,
                    model: None,
                    workdir_hash: None,
                    created_at: i64::from(n),
                    updated_at: i64::from(n),
                    summary: None,
                    summary_seq: None,
                    summary_images: vec![],
                    handoff_seq: None,
                    handoff_plan: None,
                    skill: None,
                    task_type: None,
                    requirement: None,
                })
                .await
                .unwrap();
        }
        assert_eq!(
            list_all_sessions(&store).await.unwrap().len(),
            STORE_PAGE_SIZE as usize + 1
        );
    }
}
