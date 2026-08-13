//! Keymap modal (Ctrl+H): view and re-bind all 21 global keyboard shortcuts.
//! Changes are saved as a JSON patch to `opencoder.json` and take effect
//! immediately after the menu closes.
//!
//! The modal also has a bottom button bar with **退出** (Exit),
//! **恢复默认** (Reset), and **帮助** (Help). The Reset button opens a
//! confirmation dialog; the Help button opens a shortcut-reference overlay.

pub mod help;
pub mod mouse;
pub mod state;
pub mod view;

pub use state::{handle_keymap_key, KeymapMenu, KeymapOutcome};
pub use view::render_keymap_popup;
