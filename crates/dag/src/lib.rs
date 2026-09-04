//! Pure domain layer for node-side DAG workflow scheduling (Phase: DAG).
//!
//! This crate is deliberately dependency-light (serde only) so BOTH the
//! server chain (`opencoder-web` / `opencoder-server`) and the agent chain
//! (`opencode-agent` / `opencoder-dag-runtime`) can share one source of
//! truth for:
//!
//! - [`spec`] — the persisted workflow spec (`dag_defs.spec_json`) and its
//!   snapshot copy (`dag_runs.spec_json`): step kinds, dependency edges.
//! - [`domain`] — validation (slugs, dangling deps, cycles), topological
//!   order, and the pure scheduler step: which steps are runnable given a
//!   map of finished steps.
//! - [`transitions`] — the run/step state machine. It mirrors the
//!   `NodeTaskStatus` semantics used by `node_tasks`
//!   (`pending -> running -> done|error|cancelled`, `cancelling` collapse)
//!   so claim / lost-sweep / cancel piggyback can reuse the proven
//!   node-task protocols verbatim.
//! - [`artifacts`] — the node-local `/workflow/<run_id>/<step>/` directory
//!   contract (pure path/slug/truncation helpers; no IO).
//!
//! The SERVER never executes steps: it stores specs, dispatches runs,
//! records node-uploaded events and projects UI state from them. All
//! execution and artifact writes happen on the claiming node.

pub mod artifacts;
pub mod domain;
pub mod protocol;
pub mod spec;
pub mod transitions;

pub use artifacts::{
    context_dir, meta_value, output_snapshot, run_root, step_dir, validate_step_slug,
    MAX_SNAPSHOT_BYTES,
};
pub use domain::{ready_steps, render_context, run_outcome, topo_order, validate, StepOutputs, StepStates};
pub use protocol::{
    DagClaimedRun, DagDefUpsertRequest, DagDefView, DagDispatchRequest, DagDispatchResponse,
    DagEventBatch, DagEventIn, DagEventView, DagRunView, DagStatusReport,
};
pub use spec::{DagSpec, SandboxMode, StepKind, StepSpec};
pub use transitions::{transition_allowed, DagRunStatus, StepOutcome};

#[cfg(test)]
mod tests {
    use super::*;
    use domain::StepStates;
    use serde_json::json;
    use transitions::StepOutcome;

    fn sample_spec() -> DagSpec {
        serde_json::from_value(json!({
            "name": "etl",
            "steps": [
                { "name": "fetch", "kind": { "type": "python", "code": "print(1)" } },
                { "name": "review", "depends_on": ["fetch"],
                  "kind": { "type": "agent", "prompt": "review fetch output" } },
                { "name": "load", "depends_on": ["review"],
                  "kind": { "type": "python", "code": "print(2)" } }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn sample_spec_is_valid_and_topological() {
        let spec = sample_spec();
        assert!(validate(&spec).is_ok());
        assert_eq!(topo_order(&spec).unwrap(), vec!["fetch", "review", "load"]);
    }

    #[test]
    fn ready_steps_respect_dependencies() {
        let spec = sample_spec();
        assert_eq!(ready_steps(&spec, &StepStates::new()), vec!["fetch"]);
        let after_fetch: StepStates = [("fetch".to_string(), StepOutcome::Done)]
            .into_iter()
            .collect();
        assert_eq!(ready_steps(&spec, &after_fetch), vec!["review"]);
        // A failed upstream blocks its dependents forever.
        let fetch_failed: StepStates = [("fetch".to_string(), StepOutcome::Error)]
            .into_iter()
            .collect();
        assert!(ready_steps(&spec, &fetch_failed).is_empty());
    }
}
