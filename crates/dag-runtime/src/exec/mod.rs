//! Step executors and their shared plumbing.

pub mod agent;
pub mod how_append;
pub mod runner;
pub mod wasm;

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
    /// The upstream `context` object delivered to the step (agent prompt
    /// header; wasm steps get the same object as a `context.json` file
    /// whose path arrives via `OPENCODER_STEP_CONTEXT`). Only declared
    /// upstream steps leak.
    pub fn context(&self) -> Value {
        render_context(&self.spec, &self.step.name, &self.states, &self.outputs)
    }
}

/// Terminal result of one step execution.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct StepResult {
    pub outcome: StepOutcome,
    pub error: Option<String>,
    /// Captured stdout / transcript tail (goes to `output.txt` + the
    /// truncated `step_done` event snapshot).
    pub output_text: String,
    /// Parsed `output.json` when the step produced one.
    pub output_json: Option<Value>,
    /// Session id for agent steps (None for wasm).
    pub session_id: Option<String>,
}

/// Execute an `agent` step through the real session runner.
pub use agent::execute_agent_step;

/// Execute a `wasm` step (embedded wasm runtime by default; `runc` when
/// the step opts in via `sandbox: runc`).
pub use wasm::execute_wasm_step;
