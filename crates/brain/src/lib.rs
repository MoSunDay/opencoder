//! Project "brain" — the durable goals / capability library domain layer.
//!
//! A capability records *what the project can do* (type, one-sentence summary,
//! input/output shape, exemplar engineering inputs) plus one embedding so the
//! library is semantically searchable. This crate is deliberately thin:
//!
//! - [`types`]  — the wire payload (`CapabilityInput`), pure data;
//! - [`domain`] — pure functions (validation, embed-text composition, the
//!   little-endian f32 byte codec shared with the store's vector columns);
//! - [`error`]  — typed failure markers (`EmbeddingFailed` for upstream
//!   embed outages, `BrainNotFound` for updates of an unknown id) that
//!   consumers downcast instead of string-matching error chains;
//! - [`runtime`] — I/O orchestration over `Arc<dyn Store>` + `Arc<dyn ChatStream>`.
//!
//! Both seams (`Store`, `ChatStream`) are the same abstractions the rest of
//! the workspace builds on, so storage and embedding backends stay swappable.

pub mod domain;
pub mod error;
pub mod plan;
pub mod planning;
pub mod runtime;
pub mod types;

pub use error::{BrainNotFound, EmbeddingFailed, PlanGenerationFailed, PlanNotFound};
pub use plan::{DecisionTree, DispatchOutcome, PlanNode};
pub use planning::{situation_digest, Dispatched, PLANNER_FRAMEWORK_PROMPT};
pub use runtime::Runtime;
pub use types::CapabilityInput;
