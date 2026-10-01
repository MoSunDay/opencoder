//! Integration test for the feature-gated MySQL / StarRocks project-store
//! UPGRADE path (`sql_store::ddl::upgrade`'s ADD COLUMN contract).
//!
//! The whole body lives in a `cfg`-gated module so default-feature builds
//! compile this file empty (no sqlx). The live tests read their DSN from the
//! environment (`OC_TEST_MYSQL_DSN` / `OC_TEST_STARROCKS_DSN`) and SKIP with
//! a warning when unset — no credentials ever live in the repo. When set,
//! they hand-build the PRE-EXECUTOR legacy table shape (the historical
//! column lists, hardcoded here on purpose), then require `sql_store::open`
//! (apply + upgrade) to ADD the executor columns in place: legacy fixture →
//! information_schema shows no executor columns yet → open succeeds → the
//! columns appear on both tables → executor-dimension CRUD round-trips
//! through the upgraded tables → a second open is an idempotent no-op. The
//! test DSN must point at a scratch database: the contract DROPs and
//! re-CREATEs `project_todos` / `project_todo_runs` (dropping them again at
//! the end so later runs start clean); cargo runs test binaries
//! sequentially, so this cannot race the `sql_project_store` binary.

#[cfg(any(feature = "mysql", feature = "starrocks"))]
#[path = "sql_project_upgrade/gated.rs"]
mod gated;
