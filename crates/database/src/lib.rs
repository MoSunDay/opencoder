//! Shared embedded database execution. Disk work runs outside application workers.
pub mod busy;
pub mod checkpoint;
mod engine;
mod lease;
pub mod maintenance;
pub mod maintenance_lock;
mod marker;
mod pool;
mod runtime;

pub use lease::ConnectionLease;
pub use pool::{Database, Intent};
pub use turso::{self, Connection};

/// Engine identity is independent from the application's schema watermark.
pub const STORAGE_ENGINE: &str = "turso";
pub const STORAGE_ENGINE_VERSION: &str = "0.9.0-pre.2";
