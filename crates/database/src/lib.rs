//! Shared embedded database execution. Disk work runs outside application workers.
pub mod maintenance;
mod marker;
mod pool;
mod runtime;

pub use pool::Database;
pub use turso::{self, Connection};

/// Engine identity is independent from the application's schema watermark.
pub const STORAGE_ENGINE: &str = "turso";
pub const STORAGE_ENGINE_VERSION: &str = "0.8.3";
