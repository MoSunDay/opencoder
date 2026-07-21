//! `/config` and `/model` modals for the TUI.
//!
//! - `/config` — generation-parameter form (reasoning, interleave, max_tokens,
//!   threshold, fps, capabilities). No model/base_url/api_key — those moved
//!   to `/model`.
//! - `/model` — provider CRUD: list (select/edit/add/delete) and add/edit form
//!   (name, model_id, base_url, api_key, custom headers).
//!
//! The top-level dispatch is in [`state`]: `handle_model_key` routes to the
//! per-mode handler. Each mode takes ownership of its form, mutates it, and
//! returns `(ModelOutcome, Option<ModelMenu>)`. Rendering dispatch is in
//! [`view`]. The menus own no I/O — they return a JSON merge-patch and the
//! caller (`app.rs`) persists it via `Config::save`.

pub mod config_form;
pub mod headers;
pub mod list;
pub mod patch;
pub mod provider_form;
pub mod state;
pub mod view;

pub use config_form::{ConfigField, ConfigForm, Reasoning};
pub use headers::HeadersEditor;
pub use list::{ProviderEntry, ProviderList};
pub use patch::{ConfigPatch, ProviderPatch};
pub use provider_form::{ProviderField, ProviderForm};
pub use state::{handle_model_key, mask_key, ModelMenu, ModelOutcome};
pub use view::render_model_popup;

#[cfg(test)]
mod tests;
