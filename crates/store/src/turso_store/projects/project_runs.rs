//! Project-module persistence — todos & todo runs CRUD (turso).
//!
//! Companion of [`super::project`] (which holds goals/initiatives and the
//! `ProjectStore` impl). Free functions over a raw `Connection`; deletes run
//! via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`) with explicit cascades.

use anyhow::{Context, Result};

use turso::{params, Connection, Value};

use crate::project_types::{
    ProjectExecutorKind, ProjectTodoRunKind, ProjectTodoRunPatch, ProjectTodoRunRecord,
    ProjectTodoRunStatus, ProjectTodoStatus,
};
include!("project_runs/parts/part_1.rs");

#[path = "project_runs/board.rs"]
mod board;

#[path = "project_runs/summary.rs"]
mod summary;

pub use board::reorder_todos;

use summary::summary_text;

#[path = "project_runs/todo_patch.rs"]
mod todo_patch;

#[path = "project_runs/todos.rs"]
mod todos;

pub use todos::*;
include!("project_runs/parts/part_2.rs");

include!("project_runs/parts/part_3.rs");
