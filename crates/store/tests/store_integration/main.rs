//! P0 functional tests for the libsql-backed Store.
//!
//! Each test asserts a *behavior contract*, not "the function runs":
//! - create_get_update_delete_session_contract: full CRUD lifecycle
//! - clear_other_sessions_keeps_current_and_cascades: keep-one cleanup + FK cascade
//! - append_and_load_preserves_all_roles_and_blocks: roles/blocks/usage round-trip
//! - jsonl_import_roundtrip: import preserves message history + idempotent re-run
//! - transaction_rollback_on_partial_failure: failed batch leaves no partial rows
//! - list_pagination_with_metadata: cursor pagination + search filter
//! - bundle_export_import_roundtrip: binary bundle export/import incl. subagents
//! - session_handoff_and_skill_fields_round_trip: v3 session fields via patch
//! - cancelled_transaction_*: future-cancellation must not panic and the store
//!   stays usable/consistent afterwards
//!
//! These run against a real on-disk libsql file (tempdir) so WAL behaviour is
//! exercised truthfully, not mocked. Concurrent-writer stress tests live in
//! `store_concurrency.rs`, schema-migration tests in `store_migrations.rs`, and
//! subagent-task tests in `subagent_status_counts.rs`.
//!
//! Module layout (one file per responsibility, shared helpers in `common`):
//! - `sessions` - session CRUD / patch / clear / handoff-skill field contracts
//! - `messages` - message append/load and JSONL import round-trips
//! - `transactions` - atomicity, rollback and cancellation-safety contracts
//! - `listing` - list_sessions pagination, filtering and subagent visibility
//! - `bundle` - binary bundle export/import round-trip
//! - `events` - session events, delivery enum and last_message_seq tracking

mod bundle;
mod common;
mod events;
mod listing;
mod messages;
mod sessions;
mod transactions;
