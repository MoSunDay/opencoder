//! Persistence seam for the event framework.
//!
//! [`EventLedger`] is the only storage surface this crate sees: the event
//! log (append plus cursor reads), listener registry CRUD, and pause-wait
//! registrations. [`MemoryLedger`] implements the trait with plain maps and
//! is the reference semantics for tests; this crate never
//! links a storage engine.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::domain::consumer::{validate_consumer_id, validate_cursor};
use crate::domain::event::{EventKind, EventRecord};
use crate::domain::listener::{validate, validate_kind_pattern, ListenerSpec};
use crate::domain::registry::matches;

/// Read/write surface every ledger backend must provide.
#[async_trait]
pub trait EventLedger: Send + Sync {
    /// Appends an event to the durable log.
    async fn append(&self, record: &EventRecord) -> anyhow::Result<()>;
    /// Reads up to `limit` events with `seq > after_seq`, ordered by `seq`.
    async fn events_after(&self, after_seq: i64, limit: u32) -> anyhow::Result<Vec<EventRecord>>;
    /// Upserts a listener spec (validated first).
    async fn save_listener(&self, spec: &ListenerSpec) -> anyhow::Result<()>;
    /// Removes a listener; removing an unknown id succeeds.
    async fn delete_listener(&self, id: &str) -> anyhow::Result<()>;
    /// Lists listeners ordered by id.
    async fn list_listeners(&self) -> anyhow::Result<Vec<ListenerSpec>>;
    /// Registers that `execution_id` waits for events matching
    /// `kind_pattern` (exact kind or trailing-`*` glob).
    async fn save_wait(&self, execution_id: &str, kind_pattern: &str) -> anyhow::Result<()>;
    /// Drops an execution's wait registration; idempotent.
    async fn clear_wait(&self, execution_id: &str) -> anyhow::Result<()>;
    /// Execution ids whose wait pattern matches `kind`, ordered by id.
    async fn due_waits(&self, kind: &EventKind) -> anyhow::Result<Vec<String>>;
    /// Durable replay cursor of `consumer_id`; `None` = never saved.
    /// The id is validated (non-empty, bounded, key-safe) before reading.
    async fn consumer_cursor(&self, consumer_id: &str) -> anyhow::Result<Option<i64>>;
    /// Persists the replay cursor of `consumer_id` (validated id,
    /// non-negative seq). Saving is the consumer's commit point: only call
    /// it after every action of a batch succeeded.
    async fn save_consumer_cursor(&self, consumer_id: &str, seq: i64) -> anyhow::Result<()>;
}

#[derive(Debug, Default)]
struct LedgerState {
    events: Vec<EventRecord>,
    listeners: BTreeMap<String, ListenerSpec>,
    waits: BTreeMap<String, String>,
    cursors: BTreeMap<String, i64>,
}

/// In-memory [`EventLedger`]: maps under an async mutex, sorted reads.
#[derive(Debug, Default)]
pub struct MemoryLedger {
    state: tokio::sync::Mutex<LedgerState>,
}

impl MemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EventLedger for MemoryLedger {
    async fn append(&self, record: &EventRecord) -> anyhow::Result<()> {
        self.state.lock().await.events.push(record.clone());
        Ok(())
    }

    async fn events_after(&self, after_seq: i64, limit: u32) -> anyhow::Result<Vec<EventRecord>> {
        let state = self.state.lock().await;
        let mut matched: Vec<EventRecord> = state
            .events
            .iter()
            .filter(|record| record.seq > after_seq)
            .cloned()
            .collect();
        matched.sort_by_key(|record| record.seq);
        matched.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(matched)
    }

    async fn save_listener(&self, spec: &ListenerSpec) -> anyhow::Result<()> {
        validate(spec)?;
        self.state
            .lock()
            .await
            .listeners
            .insert(spec.id.clone(), spec.clone());
        Ok(())
    }

    async fn delete_listener(&self, id: &str) -> anyhow::Result<()> {
        self.state.lock().await.listeners.remove(id);
        Ok(())
    }

    async fn list_listeners(&self) -> anyhow::Result<Vec<ListenerSpec>> {
        Ok(self
            .state
            .lock()
            .await
            .listeners
            .values()
            .cloned()
            .collect())
    }

    async fn save_wait(&self, execution_id: &str, kind_pattern: &str) -> anyhow::Result<()> {
        if execution_id.trim().is_empty() {
            anyhow::bail!("execution id must be non-empty");
        }
        validate_kind_pattern(kind_pattern)?;
        self.state
            .lock()
            .await
            .waits
            .insert(execution_id.to_string(), kind_pattern.to_string());
        Ok(())
    }

    async fn clear_wait(&self, execution_id: &str) -> anyhow::Result<()> {
        self.state.lock().await.waits.remove(execution_id);
        Ok(())
    }

    async fn due_waits(&self, kind: &EventKind) -> anyhow::Result<Vec<String>> {
        let state = self.state.lock().await;
        Ok(state
            .waits
            .iter()
            .filter(|(_, pattern)| matches(pattern, kind.as_str()))
            .map(|(execution_id, _)| execution_id.clone())
            .collect())
    }

    async fn consumer_cursor(&self, consumer_id: &str) -> anyhow::Result<Option<i64>> {
        validate_consumer_id(consumer_id).map_err(|e| anyhow::anyhow!("{e}"))?;
        let state = self.state.lock().await;
        Ok(state.cursors.get(consumer_id).copied())
    }

    async fn save_consumer_cursor(&self, consumer_id: &str, seq: i64) -> anyhow::Result<()> {
        validate_consumer_id(consumer_id).map_err(|e| anyhow::anyhow!("{e}"))?;
        validate_cursor(seq).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.state
            .lock()
            .await
            .cursors
            .insert(consumer_id.to_string(), seq);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seq: i64) -> EventRecord {
        crate::domain::event::new_record(
            crate::domain::event::EventKind::new("event.custom.tick"),
            serde_json::Value::Null,
            "test",
            seq,
            chrono::Utc::now(),
        )
    }

    #[tokio::test]
    async fn consumer_cursors_are_independent_and_validated() {
        let ledger = MemoryLedger::new();
        assert_eq!(ledger.consumer_cursor("pump-a").await.unwrap(), None);

        ledger.save_consumer_cursor("pump-a", 5).await.unwrap();
        ledger.save_consumer_cursor("pump-b", 2).await.unwrap();
        assert_eq!(ledger.consumer_cursor("pump-a").await.unwrap(), Some(5));
        assert_eq!(ledger.consumer_cursor("pump-b").await.unwrap(), Some(2));

        // Overwrite moves the same consumer forward.
        ledger.save_consumer_cursor("pump-a", 9).await.unwrap();
        assert_eq!(ledger.consumer_cursor("pump-a").await.unwrap(), Some(9));

        for bad in ["", "id with space", "a/b"] {
            assert!(
                ledger.save_consumer_cursor(bad, 1).await.is_err(),
                "{bad:?}"
            );
            assert!(ledger.consumer_cursor(bad).await.is_err(), "{bad:?}");
        }
        let too_long = "a".repeat(crate::domain::consumer::MAX_CONSUMER_ID_BYTES + 1);
        assert!(ledger.save_consumer_cursor(&too_long, 1).await.is_err());
        assert!(ledger.save_consumer_cursor("pump-a", -1).await.is_err());
        assert_eq!(ledger.consumer_cursor("pump-a").await.unwrap(), Some(9));
    }

    #[tokio::test]
    async fn events_after_reads_strictly_after_cursor() {
        let ledger = MemoryLedger::new();
        for seq in 1..=4 {
            ledger.append(&record(seq)).await.unwrap();
        }
        assert_eq!(ledger.events_after(0, 10).await.unwrap().len(), 4);
        assert_eq!(ledger.events_after(2, 10).await.unwrap().len(), 2);
        assert!(ledger.events_after(4, 10).await.unwrap().is_empty());
    }
}
