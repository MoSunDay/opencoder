//! Live event bus: fan-out of published events to current subscribers.
//!
//! The bus is a thin `tokio::sync::broadcast` hub around
//! [`EventRecord`]. It keeps no replay history: durability and cursor reads
//! belong to the [`crate::ledger`] `EventLedger`. Subscribers that fall
//! behind by more than the channel capacity receive a `Lagged` error and
//! continue from the retained tail; slow readers never block fast ones.

use tokio::sync::broadcast;

use crate::domain::event::EventRecord;

/// Default hub capacity (retained records per subscriber window).
pub const DEFAULT_BUS_CAPACITY: usize = 1024;

/// Thin handle over a broadcast hub; clone-cheap and domain-logic free.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<EventRecord>,
}

impl EventBus {
    /// Builds a hub retaining at least `capacity` records per subscriber;
    /// capacities below one are clamped to one.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Builds a hub with [`DEFAULT_BUS_CAPACITY`].
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_BUS_CAPACITY)
    }

    /// Fans a record out to every current subscriber; returns the number of
    /// subscribers that received it (zero when nobody listens).
    pub fn publish(&self, record: &EventRecord) -> usize {
        self.sender.send(record.clone()).unwrap_or_default()
    }

    /// Subscribes to future publishes; no replay of past records.
    pub fn subscribe(&self) -> broadcast::Receiver<EventRecord> {
        self.sender.subscribe()
    }

    /// Current subscriber count.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}
