use super::*;
use crate::fleet::{ExecutionKind, ExecutionStatus};
fn row(id: &str) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at: 1,
        kind: ExecutionKind::Agent,
        node_id: "node-test".into(),
        status: ExecutionStatus::Done,
    }
}

#[test]
fn idle_changes_and_cursor_expiry_preserve_inventory() {
    let mut log = InventoryLog::default();
    for id in 0..20_000 {
        assert!(log.set(row(&format!("agent-{id}"))));
    }
    let revision = log.revision();
    assert!(!log.set(row("agent-1")));
    assert!(log.read(Some(revision)).records.is_empty());
    let mut changed = row("agent-1");
    changed.status = ExecutionStatus::Running;
    log.set(changed.clone());
    log.remove("agent-2");
    let delta = log.read(Some(revision));
    assert!(!delta.full);
    assert_eq!(delta.records, vec![changed]);
    assert_eq!(delta.removed_ids, vec!["agent-2"]);
    for n in 0..HISTORY_LIMIT {
        let mut changed = row("agent-1");
        changed.created_at = n as i64;
        log.set(changed);
    }
    assert!(log.read(Some(revision)).full);
    assert_eq!(log.read(None).records.len(), 19_999);
}

#[test]
fn pages_hold_their_snapshot_while_source_changes() {
    let mut log = InventoryLog::default();
    for id in 0..600 {
        log.set(row(&format!("agent-{id:04}")));
    }
    let mut pages = InventoryPages::default();
    let mut page = pages.begin("generation-a", log.read(None));
    let original = page.revision;
    log.remove("agent-0599");
    log.set(row("agent-new"));
    let mut collected = Vec::new();
    loop {
        assert_eq!(page.revision, original);
        assert!(page.records.len() <= 256);
        collected.extend(page.records);
        let Some(next) = page.next else { break };
        assert!(pages.next("generation-b", &next).is_err());
        page = pages.next("generation-a", &next).unwrap();
    }
    assert_eq!(collected.len(), 600);
    assert_eq!(collected.last().unwrap().id, "agent-0599");
    let delta = log.read(Some(original));
    assert_eq!(delta.records[0].id, "agent-new");
    assert_eq!(delta.removed_ids, vec!["agent-0599"]);
    assert!(pages.next("generation-a", "bad").is_err());
}
