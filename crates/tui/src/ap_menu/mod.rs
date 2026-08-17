//! `/ap` modal — pick the autopilot mode (off / ap / review) and persist it
//! as a `{"autopilot":{"mode":..}}` merge-patch. Mirrors the `/skill` menu:
//! menus own no I/O; they return a JSON merge-patch the caller persists via
//! `Config::save`.

pub mod list;
pub mod state;
pub mod view;

pub use list::{ap_mode_json, chip_label, mode_index, ApChoice, AP_CHOICES};
pub use state::{handle_ap_key, ApMenu, ApOutcome};
pub use view::render_ap_popup;
