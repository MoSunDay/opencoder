//! Node-owned execution adapters. The old Web handlers are an in-process
//! session API here; no listener or server-side session store is involved.
mod brain;
mod inventory;
mod journal;
mod layout;
mod lifecycle;
mod migration;
mod migration_io;
mod operations;
mod resources;
pub mod result_reader;
pub use resources::requires_agent_pool;
mod runtime;
mod service;
mod state;
mod workloads;
pub use layout::DirectoryLayout;
pub use migration::{migrate_layout, MigrationReport};
/// Linux identity provisioning surface for cross-crate integration tests
/// (real useradd/userdel round-trips; see tests/operator_linux_identity.rs).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod linux_identity {
    pub use crate::operations::operator_env::linux_identity::{
        admission, admit, ensure_capability, lookup, provision, release, username_for,
        LinuxIdentity,
    };
    pub use crate::operations::operator_env::linux_identity::tiers::profile_ownership;
    pub use crate::operations::operator_env::linux_identity::sweep::sweep_stale;
    pub use crate::operations::operator_env::linux_identity::tiers::{
        ensure_read_only_group_gid, grant_group_read, read_only_roots, READ_ONLY_GROUP,
    };
    pub use crate::operations::operator_env::{
        env_pairs, fork_environment, identity_annotation, materialize, release_identity,
        OperatorEnvironment,
    };
}
pub use runtime::{DrainPolicy, HealthReader, HostBinding, StorageCapacity, WorkerRuntime};
pub use state::{Worker, WorkerOptions};

mod maintenance_tools;
