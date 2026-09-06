#[cfg(feature = "runtime")]
mod batch;
pub mod domain;
#[cfg(feature = "runtime")]
pub mod execution;
#[cfg(feature = "runtime")]
mod json_output;
#[cfg(feature = "runtime")]
pub mod parent;
#[cfg(feature = "runtime")]
pub mod persistence;
#[cfg(feature = "runtime")]
pub mod runner;
#[cfg(feature = "runtime")]
pub mod transitions;
pub mod types;

#[cfg(feature = "runtime")]
pub use runner::{interrupt, Runtime};
pub use types::{WorkflowSpec, WorkflowState};

pub fn parse_spec(input: &str) -> anyhow::Result<WorkflowSpec> {
    let spec: WorkflowSpec = serde_json::from_str(input)?;
    domain::validate_spec(&spec)?;
    Ok(spec)
}
