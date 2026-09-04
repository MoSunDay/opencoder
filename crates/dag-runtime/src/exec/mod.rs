//! Step executors and their shared plumbing.

pub mod agent;
pub mod python;

use std::path::PathBuf;
use std::sync::Arc;

use opencoder_dag::{render_context, DagSpec, StepOutcome, StepOutputs, StepSpec, StepStates};
use opencoder_llm::ChatStream;
use opencoder_store::Store;
use serde_json::Value;

/// Everything step execution needs that does not change per step (mirrors
/// the node-task executor's `ExecDeps`).
pub struct ExecDeps {
    pub store: Arc<dyn Store>,
    pub client: Arc<dyn ChatStream>,
    pub workdir: PathBuf,
    pub config: opencoder_core::Config,
}

/// Pure per-step execution context handed to the executors.
pub struct StepCtx {
    pub run_id: String,
    pub spec: DagSpec,
    pub step: StepSpec,
    pub states: StepStates,
    pub outputs: StepOutputs,
    pub workflow_root: PathBuf,
}

impl StepCtx {
    /// The upstream `context` object injected into the step (python global
    /// `context`; agent prompt header). Only declared upstream steps leak.
    pub fn context(&self) -> Value {
        render_context(&self.spec, &self.step.name, &self.states, &self.outputs)
    }
}

/// Terminal result of one step execution.
pub struct StepResult {
    pub outcome: StepOutcome,
    pub error: Option<String>,
    /// Captured stdout / transcript tail (goes to `output.txt` + the
    /// truncated `step_done` event snapshot).
    pub output_text: String,
    /// Parsed `output.json` when the step produced one.
    pub output_json: Option<Value>,
    /// Session id for agent steps (None for python).
    pub session_id: Option<String>,
}

/// Execute an `agent` step through the real session runner.
pub use agent::execute_agent_step;

/// Execute a `python` step (embedded VM by default; `runc` when the step
/// opts in via `sandbox: runc`).
pub use python::execute_python_step;
