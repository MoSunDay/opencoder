//! Execution-ledger domain: the brain capability library, the worker-node
//! registry and (in later assembly steps) layered runs, the node-task
//! queue, DAG runs, schedules, team runs, reporting outboxes and inventory.

pub mod brain;
pub mod nodes;

pub mod brain_layered;
pub mod dag;
pub mod dag_events;
pub mod dag_snapshot;
pub mod inventory;
pub mod node_state;
pub mod node_tasks;
pub mod reporting;
pub mod schedule;
pub mod team_runs;
