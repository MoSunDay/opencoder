//! Node-owned v3 projection. Root journal holds recoverable creation intents;
//! the scheduler tables hold indexes, fences and events only.
mod api;
mod outbox;
pub(super) mod output;
mod run;
mod state;
pub use api::handle;
pub use outbox::frames;
pub use run::{recover, run};
