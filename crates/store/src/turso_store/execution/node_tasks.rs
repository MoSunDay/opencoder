//! Node task queue (`node_tasks`) — dispatch, claim, transitions, lost-node convergence.
//!
//! Free functions over a raw `Connection`, mirroring sibling submodules;
//! multi-statement ops run via [`super::tx::run_tx`] (`BEGIN IMMEDIATE`).

use anyhow::{Context, Result};

use turso::{params, Connection};

use super::node_state::transition_allowed;

use crate::types::{ClearNodeDialogs, NodeTaskRecord, NodeTaskStatus, SessionMeta, TASK_TYPE_NODE};
include!("node_tasks/parts/part_1.rs");

include!("node_tasks/parts/part_2.rs");
