//! `/cli` modal for managing system-prompt CLI registrations.

mod content_dialog;
mod form;
mod list;
mod state;
mod view;

pub use content_dialog::{ContentDialog, ContentOutcome};
pub use form::{CliField, CliForm};
pub use list::{CliEntry, CliList};
pub use state::{handle_cli_key, CliMenu, CliOutcome};
pub use view::{render_cli_popup, render_content_dialog};
