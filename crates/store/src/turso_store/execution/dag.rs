//! DAG workflow store (`dag_defs` / `dag_runs`) — definitions and the
//! dispatched-run lifecycle: dispatch, FIFO claim, transitions, cancel,
//! lost-node convergence. Free functions over a raw `Connection`, mirroring
//! sibling submodules; multi-statement ops run via [`super::tx::run_tx`]
//! (`BEGIN IMMEDIATE`). Claim / cancel-piggyback / lost-sweep are
//! transplanted from `node_tasks` (grid: `opencoder_dag::transitions`).
//! The append-only event stream lives in [`super::dag_events`].

use anyhow::{Context, Result};

use turso::{params, params::IntoParams, Connection};

use opencoder_dag::{transition_allowed, DagRunStatus};

use crate::types::{ConvergedDagRun, DagDefRecord, DagRunRecord};
include!("dag/parts/part_1.rs");

include!("dag/parts/part_2.rs");
