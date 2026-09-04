//! `opencode-agents` — the **write path** for file-based custom agents
//! (`~/.opencoder/agents/`). The read side (resolution, composition,
//! listings) lives in `opencoder_core::agent`; this crate owns every
//! mutation of that tree:
//!
//! - **Versioned resource pools** ([`write::save_resource_version`]): `prompts|skills|tools|memory/<name>/v{n}/…` written via a temp dir + atomic rename, so a crashed writer can never publish a torn version. Version numbers are *never reused* — `next` is `max(history ∪ {current}) + 1`, so a rollback followed by a save still lands on a fresh number.
//! - **Reference cards** ([`write::create_agent`], [`write::update_agent_refs`], [`write::delete_agent`]): `<agent>/meta.json` names pool resources; updates append one history entry per *changed* field and refresh the resolved `references` snapshot.
//! - **Rollback** ([`rollback::rollback_resource`]): pointer-only switch of a pool's `current` back to a historical version — version dirs are never deleted.
//! - **Tool surface** ([`tools_paths::tools_paths`]): pure delegation to the core read path so session/web never touch layout internals.
//!
//! Everything on disk is replaced atomically (temp sibling + fsync +
//! rename; 0o600 on unix) via [`io::atomic_write`] — mirroring the envs/
//! active-marker writer in `opencoder_core::config::envs`.
//!
//! Pure-functional style: free functions over plain structs, no classes,
//! no interior state — the process-global agents-root override (used by
//! tests) is owned by `opencoder_core::agent::meta`.

pub mod io;
pub mod nfs;
pub mod references;
pub mod rollback;
pub mod serve;
pub mod tools_paths;
pub mod write;

pub use io::{atomic_write, atomic_write_json, now_rfc3339};
/// Read-only NFSv3 export of the agents root (`agent.nfs` config block):
/// the VFS lives in [`nfs`], the server handle/status in [`serve`].
pub use nfs::{agents_fs, ReadOnlyAgentsFs};
pub use references::{references_snapshot, refresh_agent_references, scan_resource};
pub use rollback::rollback_resource;
pub use serve::{
    default_opts_from_config, nfs_status, spawn_nfs_server, NfsServerHandle, NfsServerOpts,
    NfsServerStatus,
};
pub use tools_paths::tools_paths;
pub use write::{
    create_agent, delete_agent, save_resource_version, update_agent_refs, VersionFile,
};

#[cfg(test)]
pub(crate) mod testutil;
