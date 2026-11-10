//! Standard fixture module for the session integration tests (rules/11).
//!
//! Naming standard for NEW test files: declare `mod support;` and reuse these
//! pure-function fixtures instead of re-declaring local copies. The legacy
//! `common/` and `harness/` modules stay as-is for the files that already use
//! them; they are not migrated wholesale — existing files adopt this module
//! when touched.
//!
//! Fixtures are plain functions returning freshly built values (no classes,
//! no shared mutable state), so each test binary links its own copy without
//! cross-test interference.

#![allow(dead_code)] // each consuming test binary uses a subset of the helpers

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::ChatStream;
use opencoder_session::SessionState;
use opencoder_store::{Store, TursoStore};

/// In-memory SQLite store: zero file residue, one per test.
pub async fn memory_store() -> Arc<dyn Store> {
    Arc::new(TursoStore::open_memory().await.unwrap())
}

/// Monotonic session-id counter: two `agent_with` calls in one test must never
/// produce the same id (session ids are store keys and history anchors).
static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

/// A `SessionState` bound to `mock` in a fresh tempdir workdir with the
/// default config and the `act` agent, plus the `TempDir` keeping the workdir
/// alive for the duration of the test.
///
/// No store is set: callers chain `.with_store(memory_store().await)` (and
/// seed the session row) themselves when the test needs persistence.
pub fn agent_with(mock: Arc<dyn ChatStream>) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let agent = resolve_agent("act").unwrap();
    let n = SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let s = SessionState::new(
        format!("sess-{n}"),
        agent,
        Config::default(),
        mock,
        dir.path().to_path_buf(),
    );
    (dir, s)
}
