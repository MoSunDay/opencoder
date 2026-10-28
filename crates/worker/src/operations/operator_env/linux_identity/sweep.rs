//! Opportunistic stale-identity sweep.
//!
//! Accounts are released on the terminal launch path, but queue-side
//! terminal transitions and node restarts can leave an inert `ocop-*`
//! account behind. Before provisioning a fresh identity, release every
//! account whose execution directory is gone or whose persisted record is
//! terminal. Best-effort: failures warn and never block admission.

use super::{account_lock, release_account, username_for};
use crate::layout::{DirectoryLayout, ALL_KINDS};
use opencoder_core::fleet::ExecutionStatus;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Minimal projection of the persisted record — everything else in
/// `execution.json` is irrelevant to staleness.
#[derive(Deserialize)]
struct RecordStatus {
    assignment: AssignmentStatus,
}

#[derive(Deserialize)]
struct AssignmentStatus {
    index: IndexStatus,
}

#[derive(Deserialize)]
struct IndexStatus {
    status: ExecutionStatus,
}

/// `Some(true)` when the persisted record reached a terminal state;
/// `None` (unknown → keep the account) when absent or unreadable.
fn record_is_terminal(path: &std::path::Path) -> Option<bool> {
    let body = std::fs::read(path).ok()?;
    let record: RecordStatus = serde_json::from_slice(&body).ok()?;
    Some(record.assignment.index.status.terminal())
}

/// Accounts still backing a live (or freshly created, record-less)
/// execution: every present execution directory whose record is not
/// terminal. `username_for` failures (foreign ids) are skipped.
fn live_usernames(layout: &DirectoryLayout) -> BTreeSet<String> {
    let mut live = BTreeSet::new();
    for kind in ALL_KINDS {
        let Ok(entries) = std::fs::read_dir(layout.kind_root(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let terminal = layout
                .record_path(kind, &id)
                .ok()
                .and_then(|path| record_is_terminal(&path));
            if terminal == Some(true) {
                continue;
            }
            if let Ok(name) = username_for(&id) {
                live.insert(name);
            }
        }
    }
    live
}

/// `ocop-*` accounts from `/etc/passwd` whose home lives below this node's
/// data root (a node never touches another node's accounts), with the home
/// directory attached.
fn provisioned_accounts(node_root: &std::path::Path) -> Vec<(String, PathBuf)> {
    let Ok(passwd) = std::fs::read_to_string("/etc/passwd") else {
        return vec![];
    };
    passwd
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?.to_owned();
            if !name.starts_with("ocop-") || !super::valid_username(&name) {
                return None;
            }
            let home = fields.nth(4).map(PathBuf::from)?;
            home.starts_with(node_root).then_some((name, home))
        })
        .collect()
}

/// Release stale accounts; idempotent and failure-tolerant.
pub fn sweep_stale(layout: &DirectoryLayout) {
    let stale: Vec<(String, PathBuf)> = {
        let live = live_usernames(layout);
        provisioned_accounts(layout.root())
            .into_iter()
            .filter(|(name, _)| !live.contains(name))
            .collect()
    };
    if stale.is_empty() {
        return;
    }
    let _guard = account_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (name, home) in stale {
        let workspace = home
            .parent()
            .map(|parent| parent.join("workspace"))
            .unwrap_or_else(|| home.clone());
        if let Err(error) = release_account(&name, &home, &workspace) {
            tracing::warn!(user = %name, %error, "stale operator identity release failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::ExecutionKind;

    fn layout(root: &std::path::Path) -> DirectoryLayout {
        DirectoryLayout::new(root.to_path_buf(), None).unwrap()
    }

    #[test]
    fn terminal_records_are_detected_and_unknown_records_keep_accounts() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let terminal = layout
            .record_path(ExecutionKind::Operator, "operator-01A")
            .unwrap();
        std::fs::create_dir_all(terminal.parent().unwrap()).unwrap();
        std::fs::write(&terminal, r#"{"assignment":{"index":{"status":"done"}}}"#).unwrap();
        assert_eq!(record_is_terminal(&terminal), Some(true));

        let running = layout
            .record_path(ExecutionKind::Operator, "operator-01B")
            .unwrap();
        std::fs::create_dir_all(running.parent().unwrap()).unwrap();
        std::fs::write(&running, r#"{"assignment":{"index":{"status":"idle"}}}"#).unwrap();
        assert_eq!(record_is_terminal(&running), Some(false));

        assert_eq!(
            record_is_terminal(&layout.root().join("missing.json")),
            None
        );
    }

    #[test]
    fn live_usernames_keep_present_directories_and_drop_terminal_ones() {
        let root = tempfile::tempdir().unwrap();
        let layout = layout(root.path());
        let keep = layout
            .execution_dir(ExecutionKind::Operator, "operator-01KEEP0")
            .unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        // Record-less fresh creation: directory present, no record yet.
        let fresh = layout
            .execution_dir(ExecutionKind::Operator, "operator-01FRESH")
            .unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        // Terminal record: its account must not count as live.
        let done = layout
            .execution_dir(ExecutionKind::Operator, "operator-01DONE0")
            .unwrap();
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(
            done.join("execution.json"),
            r#"{"assignment":{"index":{"status":"cancelled"}}}"#,
        )
        .unwrap();

        let live = live_usernames(&layout);
        assert!(live.contains("ocop-operator-01keep0"));
        assert!(live.contains("ocop-operator-01fresh"));
        assert!(!live.contains("ocop-operator-01done0"));
    }
}
