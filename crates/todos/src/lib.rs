mod batch;
pub mod domain;
pub mod execution;
mod json_output;
pub mod parent;
pub mod persistence;
pub mod runner;
pub mod transitions;
pub mod types;

pub use runner::{interrupt, Runtime};
pub use types::{WorkflowSpec, WorkflowState};

pub fn parse_spec(input: &str) -> anyhow::Result<WorkflowSpec> {
    let spec: WorkflowSpec = serde_json::from_str(input)?;
    domain::validate_spec(&spec)?;
    Ok(spec)
}
