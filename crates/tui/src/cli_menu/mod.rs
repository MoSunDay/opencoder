//! `/cli` modal for managing system-prompt CLI registrations.

mod form;
mod list;
mod state;
mod view;

pub use form::{CliField, CliForm};
pub use list::{CliEntry, CliList};
pub use state::{handle_cli_key, CliMenu, CliOutcome};
pub use view::render_cli_popup;
