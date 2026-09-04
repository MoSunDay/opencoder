//! Shared test fixtures. The agents-root override is process-global
//! (`opencoder_core::agent::meta::set_agents_dir_override`), so every test
//! touching it holds ONE static lock for its whole body — mirrors
//! `crates/core/src/agent/meta/tests.rs`.

use std::sync::{Mutex, MutexGuard};

use opencoder_core::agent::set_agents_dir_override;

/// Serializes tests that touch the process-global agents-root override.
pub(crate) static OVERRIDE_LOCK: Mutex<()> = Mutex::new(());

/// Point the agents root at a fresh tempdir under the override lock. The
/// returned guard must be held for the whole test body: without it,
/// parallel tests race on the override.
pub(crate) fn scoped() -> (tempfile::TempDir, MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().unwrap();
    let guard = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_agents_dir_override(Some(dir.path().to_path_buf()));
    (dir, guard)
}
