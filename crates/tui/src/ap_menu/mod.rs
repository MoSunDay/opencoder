//! `/ap` modal — pick the autopilot mode (off / ap / review) and persist it
//! as a `{"autopilot":{"mode":..}}` merge-patch. Mirrors the `/skill` menu:
//! menus own no I/O; they return a JSON merge-patch the caller persists via
//! `Config::save`.

pub mod list;
pub mod state;
pub mod view;

pub use list::{AP_CHOICES, ApChoice, ap_mode_json, chip_label, mode_index};
pub use state::{ApMenu, ApOutcome, handle_ap_key};
pub use view::render_ap_popup;
