//! `/envs` modal — manage named env config sets (activate/create/recapture/delete).
//! Mirrors the `/mcp` menu. Menus own no mutating I/O; they return an
//! [`EnvsOutcome`] the caller (`app_loop_envs.rs`) executes against the core
//! envs API, then mirrors the `/model` full-refresh path.

pub mod form;
pub mod list;
pub mod state;
pub mod view;

pub use form::{EnvField, EnvNameForm};
pub use list::EnvsList;
pub use state::{handle_envs_key, EnvsMenu, EnvsOutcome};
pub use view::render_envs_popup;
