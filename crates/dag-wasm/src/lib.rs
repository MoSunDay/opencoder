//! `opencoder-dag-wasm` — the versioned wasm-module pool backing DAG
//! wasm steps.
//!
//! On-disk layout (root resolved per call via [`meta::wasm_root`], never
//! created by this crate):
//!
//! ```text
//! <root>/<name>/meta.json      # WasmPoolMeta — current + history
//! <root>/<name>/v{n}/wasm.bin  # module binary, fixed name
//! <root>/<name>/v{n}/meta.json # WasmVersionMeta — sha256 + size
//! ```
//!
//! The contracts are copied verbatim from the agents resource pool
//! (`opencoder-agents` write/rollback): version numbers are `u32`,
//! monotonic and NEVER reused — `next = max(history ∪ {current}) + 1`;
//! rollback is a pointer-only switch (version dirs are never deleted by
//! it); every write is atomic (temp sibling + fsync + rename), so a
//! crashed writer can never publish a torn version.
//!
//! The root is exported read-only over NFS (mirroring the agents
//! export). A node *pins* a module by materializing the pool's current
//! `wasm.bin` at `<workflow_root>/_modules/<name>.wasm`, where wasm
//! steps resolve modules by filename (`_modules` is reserved in the
//! DAG artifacts layout for exactly this library) — bumping the pool's
//! `current` is what the next workflow run observes.
//!
//! [`meta::wasm_root`] falls back to `None` by default: web/control
//! middleware injects the configured data-dir root through the
//! task-local scope ([`scope`]), the process-global override
//! ([`meta::set_wasm_dir_override`]) or `OPENCODER_DAG_WASM_DIR`.
//!
//! Pure-functional style: free functions over plain structs, no classes.

/// Hard cap on one module binary: 32 MiB — an independent limit from the
/// text resource pools, sized for wasm binaries.
pub const MAX_WASM_BYTES: usize = 32 * 1024 * 1024;

pub mod meta;
pub mod scope;
pub mod validate;
pub mod write;

pub use meta::{
    list_pools, pool_dir, read_pool_meta, read_version_meta, set_wasm_dir_override, version_dir,
    wasm_bin, wasm_root, WasmPoolMeta, WasmVersionMeta,
};
pub use validate::{validate_name, validate_wasm_bytes, validate_wasm_bytes_with_cap};
pub use write::{delete_wasm, rollback_wasm, save_wasm_version};
