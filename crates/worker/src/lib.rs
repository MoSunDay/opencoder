//! Node-owned execution adapters. The old Web handlers are an in-process
//! session API here; no listener or server-side session store is involved.
mod brain;
mod dag_wasm_pin;
mod journal;
mod layout;
mod lifecycle;
mod migration;
mod migration_io;
mod operations;
mod resources;
pub use resources::requires_agent_pool;
mod runtime;
mod service;
mod state;
mod workloads;
pub use layout::DirectoryLayout;
pub use migration::{migrate_layout, MigrationReport};
pub use runtime::{DrainPolicy, HealthReader, HostBinding, StorageCapacity, WorkerRuntime};
pub use state::{Worker, WorkerOptions};

mod maintenance_tools;
