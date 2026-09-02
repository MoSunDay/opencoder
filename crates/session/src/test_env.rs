//! Process-wide serialization for tests that touch process-global state.
//!
//! `std::env::set_var` mutates state shared by every thread in the test
//! binary and is not thread-safe. Any test that flips `HOME` /
//! `XDG_CONFIG_HOME` must hold [`env_lock`] across the whole flip window —
//! and so must any test whose assertions READ env-derived paths (e.g. the
//! skill catalog that `estimated_tokens` resolves through `skills_dir` on
//! every call). Without a shared lock a concurrent flip can straddle two
//! snapshots and turn deterministic deltas into order-dependent flakes.

use std::sync::{Mutex, MutexGuard};

pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
