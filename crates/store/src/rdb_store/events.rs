//! Platform event log, listener specs, pause waits and the scheduler gate.
//!
//! The log is append-only and ordered by a persisted global counter
//! (`evt/<seq:020>`), so replay cursors survive restarts. Listener specs are
//! keyed by id and waits by execution id; the gate is a single JSON row.
//! All mutations commit through one atomic batch under the engine write lease.

use anyhow::{ensure, Result};

use crate::rdb_store::kv::{
    decode, get_json, platform_cursor_key, platform_event_key, platform_event_prefix,
    platform_listener_key, platform_listener_prefix, platform_wait_key, platform_wait_prefix,
    scheduler_gate_key,
};
use crate::rdb_store::{seq, tx};
use opencoder_event::domain::consumer::{validate_consumer_id, validate_cursor};
use opencoder_event::domain::event::{EventKind, EventRecord};
use opencoder_event::domain::gate::SchedulerGate;
use opencoder_event::domain::listener::ListenerSpec;
use opencoder_event::domain::registry::matches;

/// Stage an event append in `batch`, allocating its replay seq from `carry`.
pub fn stage_append(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    carry: &mut seq::SeqCarry,
    record: &EventRecord,
) -> Result<EventRecord> {
    ensure!(!record.id.trim().is_empty(), "event id must be non-empty");
    ensure!(
        !record.kind.as_str().trim().is_empty(),
        "event kind must be non-empty"
    );
    let seq = seq::next_global_carried(store, batch, carry, "evt")?;
    let mut stored = record.clone();
    stored.seq = seq;
    tx::put(batch, &platform_event_key(seq), &stored)?;
    Ok(stored)
}

/// Persist one event; returns the record with its assigned seq.
pub fn append(store: &rdb::store::Store, record: &EventRecord) -> Result<EventRecord> {
    let mut batch = rocksdb::WriteBatch::default();
    let stored = stage_append(store, &mut batch, &mut seq::SeqCarry::new(), record)?;
    tx::commit(store, batch)?;
    Ok(stored)
}

/// Events with `seq > after_seq`, ascending, capped at `limit`.
pub fn after(store: &rdb::store::Store, after_seq: i64, limit: u32) -> Result<Vec<EventRecord>> {
    let mut out = Vec::new();
    if limit == 0 {
        return Ok(out);
    }
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let prefix = platform_event_prefix();
    for (key, value) in tx::scan_prefix(store, &prefix)? {
        if seq::parse_ord(&key, &prefix).is_some_and(|n| n > after_seq) {
            out.push(decode(&value)?);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

/// Last committed event seq (0 when the log is empty).
pub fn last_seq(store: &rdb::store::Store) -> i64 {
    seq::max_child(store, &platform_event_prefix())
}

/// Upsert a validated listener spec.
pub fn save_listener(store: &rdb::store::Store, spec: &ListenerSpec) -> Result<()> {
    opencoder_event::domain::listener::validate(spec)?;
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &platform_listener_key(&spec.id), spec)?;
    tx::commit(store, batch)
}

/// Remove a listener; unknown ids are a no-op.
pub fn delete_listener(store: &rdb::store::Store, id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    batch.delete(platform_listener_key(id));
    tx::commit(store, batch)
}

/// Listeners ordered by id.
pub fn listeners(store: &rdb::store::Store) -> Result<Vec<ListenerSpec>> {
    let mut out = Vec::new();
    for (_, value) in tx::scan_prefix(store, &platform_listener_prefix())? {
        out.push(decode(&value)?);
    }
    out.sort_by(|a: &ListenerSpec, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Register (or replace) the wait pattern for an execution.
pub fn save_wait(store: &rdb::store::Store, execution_id: &str, kind_pattern: &str) -> Result<()> {
    ensure!(
        !execution_id.trim().is_empty(),
        "execution id must be non-empty"
    );
    opencoder_event::domain::listener::validate_kind_pattern(kind_pattern)?;
    let mut batch = rocksdb::WriteBatch::default();
    batch.put(platform_wait_key(execution_id), kind_pattern.as_bytes());
    tx::commit(store, batch)
}

/// Drop an execution's wait; idempotent.
pub fn clear_wait(store: &rdb::store::Store, execution_id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    batch.delete(platform_wait_key(execution_id));
    tx::commit(store, batch)
}

/// Execution ids whose wait pattern matches `kind`, ordered by id.
pub fn due_waits(store: &rdb::store::Store, kind: &EventKind) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (key, value) in tx::scan_prefix(store, &platform_wait_prefix())? {
        let pattern = std::str::from_utf8(&value)?;
        if !matches(pattern, kind.as_str()) {
            continue;
        }
        let id = std::str::from_utf8(&key)?;
        if let Some(id) = id.strip_prefix("evw/") {
            out.push(id.to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Current gate; a missing row means "not paused".
pub fn load_gate(store: &rdb::store::Store) -> Result<SchedulerGate> {
    Ok(
        crate::rdb_store::kv::get_json::<SchedulerGate>(store, &scheduler_gate_key())?
            .unwrap_or_default(),
    )
}

/// Persisted gate row, `None` until a gate was ever saved — callers that
/// need the "not paused" default for dispatch decisions use [`load_gate`].
pub fn persisted_gate(store: &rdb::store::Store) -> Result<Option<SchedulerGate>> {
    crate::rdb_store::kv::get_json::<SchedulerGate>(store, &scheduler_gate_key())
}

/// Persist the gate.
pub fn save_gate(store: &rdb::store::Store, gate: &SchedulerGate) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &scheduler_gate_key(), gate)?;
    tx::commit(store, batch)
}

/// Durable replay cursor of one consumer (`evc/<consumer-id>`); `None`
/// when the consumer never saved one. The id is validated, never trusted
/// as an arbitrary key.
pub fn consumer_cursor(store: &rdb::store::Store, consumer_id: &str) -> Result<Option<i64>> {
    validate_consumer_id(consumer_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    get_json::<i64>(store, &platform_cursor_key(consumer_id))
}

/// Persist one consumer's replay cursor. Saving is the consumer's commit
/// point; the row is a single JSON value under the consumer's key.
pub fn save_consumer_cursor(store: &rdb::store::Store, consumer_id: &str, seq: i64) -> Result<()> {
    validate_consumer_id(consumer_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    validate_cursor(seq).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &platform_cursor_key(consumer_id), &seq)?;
    tx::commit(store, batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdb_store::kv::platform_cursor_prefix;
    use opencoder_event::domain::event::{new_record, EXECUTION_TERMINAL};
    use opencoder_event::domain::listener::{new_listener, TriggerAction};
    use std::sync::Arc;

    fn temp_db() -> (tempfile::TempDir, crate::rdb_store::engine::RdbDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::rdb_store::engine::RdbDb::open(dir.path().join("db")).unwrap();
        (dir, db)
    }

    fn raw(db: &crate::rdb_store::engine::RdbDb) -> Arc<rdb::store::Store> {
        db.shared()
    }

    fn record(kind: &str) -> EventRecord {
        new_record(
            EventKind::new(kind),
            serde_json::Value::Null,
            "test",
            0,
            chrono::Utc::now(),
        )
    }

    #[test]
    fn append_assigns_monotonic_seq_and_reads_after_cursor() {
        let (_dir, db) = temp_db();
        let db = raw(&db);
        let a = append(&db, &record("a")).unwrap();
        let b = append(&db, &record("b")).unwrap();
        assert_eq!((a.seq, b.seq), (1, 2));
        let rows = after(&db, 1, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind.as_str(), "b");
        assert_eq!(last_seq(&db), 2);
        assert!(after(&db, 0, 0).unwrap().is_empty());
    }

    #[test]
    fn listener_upsert_delete_and_order() {
        let (_dir, db) = temp_db();
        let db = raw(&db);
        let spec = new_listener(
            "b-listener",
            EXECUTION_TERMINAL,
            None,
            TriggerAction::Notify {
                message: "done".into(),
            },
        );
        save_listener(&db, &spec).unwrap();
        assert_eq!(listeners(&db).unwrap(), vec![spec.clone()]);
        delete_listener(&db, &spec.id).unwrap();
        delete_listener(&db, &spec.id).unwrap();
        assert!(listeners(&db).unwrap().is_empty());
    }

    #[test]
    fn waits_match_glob_and_clear() {
        let (_dir, db) = temp_db();
        let db = raw(&db);
        save_wait(&db, "e1", EXECUTION_TERMINAL).unwrap();
        save_wait(&db, "e2", "execution.*").unwrap();
        let due = due_waits(&db, &EventKind::new(EXECUTION_TERMINAL)).unwrap();
        assert_eq!(due, vec!["e1".to_string(), "e2".to_string()]);
        clear_wait(&db, "e2").unwrap();
        assert_eq!(
            due_waits(&db, &EventKind::new(EXECUTION_TERMINAL)).unwrap(),
            vec!["e1".to_string()]
        );
    }

    #[test]
    fn gate_roundtrip() {
        let (_dir, db) = temp_db();
        let db = raw(&db);
        assert!(!load_gate(&db).unwrap().paused);
        let gate = opencoder_event::domain::gate::paused("maintenance", 3);
        save_gate(&db, &gate).unwrap();
        assert_eq!(load_gate(&db).unwrap(), gate);
    }

    fn reopen_db(path: &std::path::Path) -> Arc<rdb::store::Store> {
        let db = crate::rdb_store::engine::RdbDb::open(path).unwrap();
        db.shared()
    }

    #[test]
    fn consumer_cursor_roundtrip_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let db = reopen_db(&path);
        assert_eq!(consumer_cursor(&db, "pump-a").unwrap(), None);

        save_consumer_cursor(&db, "pump-a", 7).unwrap();
        save_consumer_cursor(&db, "pump-b", 3).unwrap();
        assert_eq!(consumer_cursor(&db, "pump-a").unwrap(), Some(7));
        assert_eq!(consumer_cursor(&db, "pump-b").unwrap(), Some(3));

        // Close every handle, reopen the same directory: cursors survive.
        drop(db);
        let db = reopen_db(&path);
        assert_eq!(consumer_cursor(&db, "pump-a").unwrap(), Some(7));
        assert_eq!(consumer_cursor(&db, "pump-b").unwrap(), Some(3));

        // Moving a consumer forward overwrites its own row only.
        save_consumer_cursor(&db, "pump-a", 9).unwrap();
        assert_eq!(consumer_cursor(&db, "pump-a").unwrap(), Some(9));
        assert_eq!(consumer_cursor(&db, "pump-b").unwrap(), Some(3));
        assert_eq!(
            tx::scan_prefix(&db, &platform_cursor_prefix())
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn consumer_cursor_rejects_unsafe_ids_and_negative_seq() {
        let (_dir, db) = temp_db();
        let db = raw(&db);
        for bad in ["", "id with space", "a/b", "\tid"] {
            assert!(save_consumer_cursor(&db, bad, 1).is_err(), "{bad:?}");
            assert!(consumer_cursor(&db, bad).is_err(), "{bad:?}");
        }
        let too_long = "a".repeat(200);
        assert!(save_consumer_cursor(&db, &too_long, 1).is_err());
        assert!(save_consumer_cursor(&db, "pump-a", -1).is_err());
        assert_eq!(consumer_cursor(&db, "pump-a").unwrap(), None);
    }
}
