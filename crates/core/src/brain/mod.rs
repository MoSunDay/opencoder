//! Versioned, executable ontology and durable orchestration wire contracts.
//! These are data only; planning and state transitions live in opencoder-brain.
mod plan;
mod run;
pub use plan::*;
pub use run::*;
pub mod resources;

mod capability;
mod io;
pub mod layered;
pub use capability::*;
pub use io::*;
