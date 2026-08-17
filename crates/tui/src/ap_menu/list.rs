//! Fixed three-entry choice table for the `/ap` mode picker. Unlike
//! `/skill` (a discovery list read from disk) the choices are compile-time
//! constants — `/ap` is a picker over the tri-state `ApMode`.

use opencoder_core::ApMode;
use serde_json::{json, Value};

/// One selectable autopilot mode with its display description.
pub struct ApChoice {
    pub mode: ApMode,
    /// Lowercase config key — the serde wire format of `ApMode`.
    pub key: &'static str,
    /// Chinese one-liner shown next to the key (kept terse for the popup).
    pub description: &'static str,
}

/// The three modes in display order.
pub const AP_CHOICES: [ApChoice; 3] = [
    ApChoice {
        mode: ApMode::Off,
        key: "off",
        description: "关闭（无自动行为）",
    },
    ApChoice {
        mode: ApMode::Ap,
        key: "ap",
        description: "完全自动（PLAN→ACT→VERIFY 自驱）",
    },
    ApChoice {
        mode: ApMode::Review,
        key: "review",
        description: "自动 review（任务完成后一次 review）",
    },
];

/// Display index of `mode` within `AP_CHOICES` (used to pre-highlight the
/// current mode). Falls back to 0 — the table is exhaustive over `ApMode`.
pub fn mode_index(mode: ApMode) -> usize {
    AP_CHOICES.iter().position(|c| c.mode == mode).unwrap_or(0)
}

/// JSON merge-patch selecting `mode`: `{"autopilot":{"mode":"off|ap|review"}}`.
/// Only the `mode` key is present so a deep-merge save keeps
/// `max_iterations` / `verify_retries` intact.
pub fn ap_mode_json(mode: ApMode) -> Value {
    let key = AP_CHOICES
        .iter()
        .find(|c| c.mode == mode)
        .map(|c| c.key)
        .unwrap_or("off");
    json!({ "autopilot": { "mode": key } })
}

/// Status-chip label for `mode`: `AP` (fully automatic), `RV` (auto review),
/// `None` when off (no chip). Consumed by `render.rs`.
pub fn chip_label(mode: ApMode) -> Option<&'static str> {
    match mode {
        ApMode::Ap => Some("AP"),
        ApMode::Review => Some("RV"),
        ApMode::Off => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mode_index` positions every `ApMode` variant in display order.
    #[test]
    fn mode_index_covers_every_mode() {
        assert_eq!(mode_index(ApMode::Off), 0);
        assert_eq!(mode_index(ApMode::Ap), 1);
        assert_eq!(mode_index(ApMode::Review), 2);
    }

    /// The merge-patch round-trips through the wire keys and carries only
    /// `mode` — other autopilot keys stay untouched by a deep-merge save.
    #[test]
    fn ap_mode_json_shape_has_only_the_mode_key() {
        for choice in &AP_CHOICES {
            assert_eq!(
                ap_mode_json(choice.mode),
                json!({ "autopilot": { "mode": choice.key } })
            );
            assert_eq!(
                ap_mode_json(choice.mode)["autopilot"]
                    .as_object()
                    .unwrap()
                    .len(),
                1,
                "patch must not carry extra autopilot keys"
            );
        }
    }

    /// The status chip is tri-state: AP / RV / absent.
    #[test]
    fn chip_label_maps_tri_state() {
        assert_eq!(chip_label(ApMode::Off), None);
        assert_eq!(chip_label(ApMode::Ap), Some("AP"));
        assert_eq!(chip_label(ApMode::Review), Some("RV"));
    }
}
