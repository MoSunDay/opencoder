//! `/api/project/*` CRUD contract over the FULL app (signature middleware
//! included): goals → initiatives → todos with list/read/PATCH semantics.
//! Run-lifecycle cases live in `tests/web_project_runs.rs`; the shared
//! harness lives in `tests/support/project_app.rs`.

mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};
use support::project_app::{call, done, harness, todo_row};

// Executor-dimension write contract (P4): the create/patch bodies carry
// `executor_kind/executor_ref/executor_spec`, bad kind strings and bad
// inline specs 400 at the door, double-option null clears, and run rows
// expose the RESOLVED `executor_kind` (plan runs stay agent).

#[path = "web_project/suite_1.rs"]
mod suite_1;
#[path = "web_project/suite_2.rs"]
mod suite_2;
