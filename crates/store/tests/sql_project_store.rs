//! Integration test for the feature-gated MySQL / StarRocks project store.
//!
//! The whole body lives in a `cfg`-gated module so default-feature builds
//! compile this file empty (no sqlx). The live tests read their DSN from the
//! environment (`OC_TEST_MYSQL_DSN` / `OC_TEST_STARROCKS_DSN`) and SKIP with
//! a warning when unset — no credentials ever live in the repo. When set,
//! they run one compact CRUD contract through the `Arc<dyn ProjectStore>`
//! seam: create goal → patch → milestone → todo → run v1 via
//! next_todo_version → list orders → plan_md clear-to-NULL → expected-status
//! CAS (todo claim-rollback + terminal-run convergence) → cascade deletes.

#[cfg(any(feature = "mysql", feature = "starrocks"))]
#[path = "sql_project_store/gated.rs"]
mod gated;
