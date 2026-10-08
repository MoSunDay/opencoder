//! Node-owned execution adapters. The old Web handlers are an in-process
//! session API here; no listener or server-side session store is involved.
mod brain;
mod journal;
mod layout;
mod lifecycle;
mod migration;
mod migration_io;
mod operations;
#[path = "resources/probe.rs"]
mod resource_probe;
mod resources;
pub use resource_probe::MountHealthReader;
pub use resources::requires_agent_pool;
mod runtime;
mod service;
mod state;
mod workloads;
pub use layout::DirectoryLayout;
pub use migration::{migrate_layout, MigrationReport};
pub use runtime::crash::settle_brain_crash;
pub use runtime::{DrainPolicy, HealthReader, HostBinding, StorageCapacity, WorkerRuntime};
pub use state::{Worker, WorkerOptions};

mod maintenance_tools;
