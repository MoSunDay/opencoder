//! Node-side DAG workflow runtime: the scheduling loop plus per-kind step
//! executors. Linked ONLY by the `opencode-agent` binary chain.
//!
//! Layering:
//! - [`runtime`] — the whole-run loop: parse the claimed spec snapshot,
//!   schedule ready steps (JoinSet, bounded concurrency), fold outcomes,
//!   upload events/status upstream, honor cancel.
//! - [`exec`] — step executors: `agent` (real session runner, the node-task
//!   executor pattern) and `python` (embedded RustPython VM or `runc`
//!   sandbox).
//! - `step_io` (private module) — per-step artifact bookkeeping helpers
//!   used by the run loop: finished-step artifact writes + state folds, and
//!   terminal outcomes for steps that never ran.
//! - [`sandbox`] — OCI bundle generation + `runc` driving (pure helpers +
//!   process wrappers; only used when a python step opts into
//!   `sandbox: runc`).

pub mod dag_events;
pub mod exec;
pub mod runtime;
pub mod sandbox;

mod step_io;

pub use dag_events::{
    run_finished_event, run_started_event, step_done_event, step_started_event, RunEventSink,
    MAX_EVENTS as DAG_EVENT_BATCH_MAX, WINDOW as DAG_EVENT_BATCH_WINDOW,
};
pub use exec::{execute_agent_step, execute_python_step, ExecDeps, StepCtx, StepResult};
pub use runtime::{execute_run, RunDeps, MAX_CONCURRENT_STEPS};

pub const RUNTIME_NAME: &str = "opencode-dag-runtime";
