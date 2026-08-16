//! `/skill` modal — manage default-injection skills (enable/disable).
//! Mirrors the `/mcp` menu. Menus own no I/O; they return a JSON
//! merge-patch the caller persists via `Config::save`.

pub mod list;
pub mod state;
pub mod view;

pub use list::{SkillEntry, SkillList, toggle_skill_json};
pub use state::{SkillMenu, SkillOutcome, handle_skill_key};
pub use view::render_skill_popup;
