//! Keymap modal (Ctrl+H): view and re-bind all 18 global keyboard shortcuts.
//! Changes are saved as a JSON patch to `opencoder.json` and take effect
//! immediately after the menu closes.
//!
//! The modal also has a bottom button bar with **退出** (Exit) and
//! **帮助** (Help). The Help button opens a static shortcut-reference overlay.

pub mod help;
pub mod state;
pub mod view;

pub use state::{handle_keymap_key, KeymapMenu, KeymapOutcome};
pub use view::render_keymap_popup;
