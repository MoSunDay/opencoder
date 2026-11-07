//! Adapter: expose any [`Store`] as an [`EventLedger`].
//!
//! Upper layers already hold `Arc<dyn Store>`; this newtype lets the event
//! executor consume the same handle without a second database dependency.

use std::sync::Arc;

use async_trait::async_trait;
use opencoder_event::domain::consumer::{validate_consumer_id, validate_cursor};
use opencoder_event::EventKind;
use opencoder_event::{EventLedger, EventRecord, ListenerSpec};

use crate::store::Store;

/// Ledger view over a store handle.
#[derive(Clone)]
pub struct StoreEventLedger {
    store: Arc<dyn Store>,
}

/// Build a ledger view over `store`.
pub fn store_ledger(store: Arc<dyn Store>) -> StoreEventLedger {
    StoreEventLedger { store }
}

#[async_trait]
impl EventLedger for StoreEventLedger {
    async fn append(&self, record: &EventRecord) -> anyhow::Result<()> {
        self.store.platform_event_append(record).await.map(|_| ())
    }

    async fn events_after(&self, after_seq: i64, limit: u32) -> anyhow::Result<Vec<EventRecord>> {
        self.store.platform_events_after(after_seq, limit).await
    }

    async fn save_listener(&self, spec: &ListenerSpec) -> anyhow::Result<()> {
        self.store.platform_listener_save(spec).await
    }

    async fn delete_listener(&self, id: &str) -> anyhow::Result<()> {
        self.store.platform_listener_delete(id).await
    }

    async fn list_listeners(&self) -> anyhow::Result<Vec<ListenerSpec>> {
        self.store.platform_listeners().await
    }

    async fn save_wait(&self, execution_id: &str, kind_pattern: &str) -> anyhow::Result<()> {
        self.store
            .platform_wait_save(execution_id, kind_pattern)
            .await
    }

    async fn clear_wait(&self, execution_id: &str) -> anyhow::Result<()> {
        self.store.platform_wait_clear(execution_id).await
    }

    async fn due_waits(&self, kind: &EventKind) -> anyhow::Result<Vec<String>> {
        self.store.platform_due_waits(kind).await
    }

    async fn consumer_cursor(&self, consumer_id: &str) -> anyhow::Result<Option<i64>> {
        validate_consumer_id(consumer_id).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.store.platform_consumer_cursor(consumer_id).await
    }

    async fn save_consumer_cursor(&self, consumer_id: &str, seq: i64) -> anyhow::Result<()> {
        validate_consumer_id(consumer_id).map_err(|e| anyhow::anyhow!("{e}"))?;
        validate_cursor(seq).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.store
            .platform_save_consumer_cursor(consumer_id, seq)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_event::domain::event::{new_record, EXECUTION_TERMINAL};
    use opencoder_event::domain::listener::{new_listener, TriggerAction};

    #[tokio::test]
    async fn adapter_delegates_log_listener_and_wait() {
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(
            crate::rdb_store::RdbStore::open(dir.path().join("db"))
                .await
                .unwrap(),
        );
        let ledger = store_ledger(Arc::clone(&store));
        ledger
            .append(&new_record(
                EventKind::new(EXECUTION_TERMINAL),
                serde_json::Value::Null,
                "test",
                0,
                chrono::Utc::now(),
            ))
            .await
            .unwrap();
        assert_eq!(ledger.events_after(0, 10).await.unwrap().len(), 1);
        let spec = new_listener(
            "l1",
            EXECUTION_TERMINAL,
            None,
            TriggerAction::Notify {
                message: "m".into(),
            },
        );
        ledger.save_listener(&spec).await.unwrap();
        assert_eq!(ledger.list_listeners().await.unwrap(), vec![spec]);
        ledger.save_wait("e1", EXECUTION_TERMINAL).await.unwrap();
        assert_eq!(
            ledger
                .due_waits(&EventKind::new(EXECUTION_TERMINAL))
                .await
                .unwrap(),
            vec!["e1".to_string()]
        );
        ledger.clear_wait("e1").await.unwrap();
        assert!(ledger
            .due_waits(&EventKind::new(EXECUTION_TERMINAL))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn adapter_delegates_cursor_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store: Arc<dyn Store> =
            Arc::new(crate::rdb_store::RdbStore::open(&path).await.unwrap());
        let ledger = store_ledger(Arc::clone(&store));

        assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);
        ledger.save_consumer_cursor("pump", 4).await.unwrap();
        ledger.save_consumer_cursor("other", 2).await.unwrap();
        assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(4));

        // Same values are visible through the store seam itself.
        assert_eq!(
            store.platform_consumer_cursor("pump").await.unwrap(),
            Some(4)
        );

        // Close the store, reopen the same directory: the cursor survives.
        drop(ledger);
        drop(store);
        let store: Arc<dyn Store> =
            Arc::new(crate::rdb_store::RdbStore::open(&path).await.unwrap());
        let ledger = store_ledger(Arc::clone(&store));
        assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(4));
        assert_eq!(ledger.consumer_cursor("other").await.unwrap(), Some(2));

        ledger.save_consumer_cursor("pump", 6).await.unwrap();
        assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(6));
    }

    #[tokio::test]
    async fn adapter_validates_consumer_id_and_cursor_value() {
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(
            crate::rdb_store::RdbStore::open(dir.path().join("db"))
                .await
                .unwrap(),
        );
        let ledger = store_ledger(store);
        for bad in ["", "id with space", "a/b"] {
            assert!(
                ledger.save_consumer_cursor(bad, 1).await.is_err(),
                "{bad:?}"
            );
            assert!(ledger.consumer_cursor(bad).await.is_err(), "{bad:?}");
        }
        assert!(ledger.save_consumer_cursor("pump", -1).await.is_err());
        assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);
    }
}
