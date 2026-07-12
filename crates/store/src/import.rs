use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use opencoder_core::Message;
use tracing::{info, warn};

use crate::store::Store;
use crate::types::ImportReport;

/// One-shot migration: read every `<dir>/<session_id>.jsonl`, create a session
/// row and import its messages into `store`. Existing sessions (by id) are
/// skipped so the migration is idempotent/re-runnable.
pub async fn import_jsonl_dir<S: Store + ?Sized>(store: &S, dir: &Path) -> Result<ImportReport> {
    let mut total = ImportReport::default();
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(total),
        Err(e) => return Err(e).context("read jsonl dir"),
    };
    let mut files: Vec<PathBuf> = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    files.sort();
    for f in files {
        let session_id = match f.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if store.get_session(&session_id).await?.is_some() {
            warn!(session_id, "already imported, skipping");
            continue;
        }
        match import_jsonl_file(store, &session_id, &f).await {
            Ok(r) => {
                total.sessions += r.sessions;
                total.messages += r.messages;
                total.skipped += r.skipped;
                info!(session_id, messages = r.messages, "imported session");
            }
            Err(e) => {
                warn!(session_id, error = %e, "failed to import session, skipping");
                total.skipped += 1;
            }
        }
    }
    Ok(total)
}

async fn import_jsonl_file<S: Store + ?Sized>(
    store: &S,
    session_id: &str,
    path: &Path,
) -> Result<ImportReport> {
    let text = tokio::fs::read_to_string(path)
        .await
        .context("read jsonl")?;
    let mut msgs: Vec<Message> = Vec::new();
    let mut skipped = 0u32;
    let mut first_user_text: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Message>(line) {
            Ok(m) => {
                if first_user_text.is_none() && m.role == opencoder_core::Role::User {
                    first_user_text = Some(m.text());
                }
                msgs.push(m);
            }
            Err(_) => {
                skipped += 1;
            }
        }
    }
    if msgs.is_empty() {
        return Ok(ImportReport {
            sessions: 0,
            messages: 0,
            skipped,
        });
    }
    let now = opencoder_core::message::now_ms();
    let earliest = msgs.first().map(|m| m.created_at).unwrap_or(now);
    let meta = crate::types::SessionMeta {
        id: session_id.to_string(),
        title: first_user_text.map(|t| t.chars().take(80).collect()),
        agent: msgs
            .iter()
            .rev()
            .find(|m| m.agent.is_some())
            .and_then(|m| m.agent.clone()),
        model: msgs
            .iter()
            .rev()
            .find(|m| m.model.is_some())
            .and_then(|m| m.model.clone()),
        workdir_hash: None,
        created_at: earliest,
        updated_at: msgs.last().map(|m| m.created_at).unwrap_or(now),
        summary: None,
        summary_seq: None,
    };
    store.create_session(&meta).await?;
    store.append_messages(session_id, &msgs).await?;
    Ok(ImportReport {
        sessions: 1,
        messages: msgs.len() as u32,
        skipped,
    })
}
