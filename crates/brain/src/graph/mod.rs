//! Single pure graph kernel; model, storage, and dispatch stay at the boundaries.
mod advance;
mod causal;
mod outputs;
mod routing;
mod validate;
pub use advance::advance;
pub use outputs::{capture, output_schema};
pub use routing::{apply, pending};
pub use validate::validate;
pub const MIGRATION: &str = "brain migration required: legacy plans and runs are read-only; submit a schema_version: 2 plan through /api/brain/plan-defs and /api/brain/runs";
