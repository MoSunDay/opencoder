//! `/short_key` modal: view and re-bind all 18 global keyboard shortcuts.
//! Changes are saved as a JSON patch to `opencoder.json` and take effect
//! immediately after the menu closes.

pub mod state;
pub mod view;

pub use state::{handle_keymap_key, KeymapMenu, KeymapOutcome};
pub use view::render_keymap_popup;
