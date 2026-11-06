//! opencoder-event: the platform event framework.
//!
//! Uniform management layer for event-driven scheduling: register listeners
//! (kind match + trigger action), publish events on a broadcast bus, and
//! route triggers through an executor. Persistence (event log, listener
//! registry, pause-wait registrations) sits behind the [`ledger`]
//! `EventLedger` trait so any key-value store can implement it.
//!
//! The crate is pure by default (no storage engine); the optional `rdb`
//! feature adds [`queue`], a durable RocksDB ledger for the event queue
//! that no other crate links.
//!
//! The crate is deliberately backend-agnostic: domain types and registry
//! math are pure functions; the bus and executor are thin runtimes.

pub mod bus;
pub mod consumer;
pub mod domain;
pub mod executor;
pub mod ledger;
#[cfg(feature = "rdb")]
pub mod queue;

pub use consumer::{PumpFailure, PumpRequest, PumpRun, run_pump};
pub use domain::{EventKind, EventRecord, ListenerSpec, SchedulerGate, TriggerAction};
pub use ledger::{EventLedger, MemoryLedger};
#[cfg(feature = "rdb")]
pub use queue::RdbQueueLedger;
