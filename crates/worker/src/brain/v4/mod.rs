//! Schema version 4: the layered capability canvas, node side.
//!
//! The root journal owns the frozen request and the finite activation markers;
//! the layered store tables own the run phase, the generation fence, the
//! operation indexes and the events. Layers are recomputed from the plan with
//! `opencoder_brain::layered::layers` and are never persisted.
pub(crate) mod api;
pub(crate) mod history;
pub(crate) mod outbox;
pub(crate) mod output;
pub(crate) mod parent;
pub(crate) mod run;
pub(crate) mod state;

pub use api::handle;
pub use outbox::frames;
pub use run::{recover, run};
