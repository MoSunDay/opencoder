use super::*;

#[test]
fn flash_visibility_expires_at_tick_boundary() {
    assert!(flash_visible(10, 24, 15));
    assert!(!flash_visible(10, 25, 15));
}

#[test]
fn flash_visibility_handles_tick_wraparound() {
    assert!(flash_visible(u32::MAX - 2, 1, 5));
    assert!(!flash_visible(u32::MAX - 2, 2, 5));
}

#[test]
fn flash_status_uses_the_same_visibility_rule() {
    let flash = Some(("ready".to_string(), 100));
    assert_eq!(flash_status_text(&flash, 114), Some("ready"));
    assert_eq!(flash_status_text(&flash, 115), None);
}

#[test]
fn warn_flash_hue_covers_plan_act_and_clear_guard() {
    assert!(super::is_warn_flash("\u{2192} plan mode"));
    assert!(super::is_warn_flash("\u{2192} edit plan"));
    assert!(super::is_warn_flash(
        "\u{2192} \u{280b} 5s \u{4e4b}\u{540e}\u{4ec5}\u{4fdd}\u{7559}\u{8ba1}\u{5212}\u{5e76}\u{6267}\u{884c}\u{2026}"
    ));
    assert!(
        !super::is_warn_flash("\u{2192} clear 5s \u{540e}\u{6e05}\u{7a7a}\u{4e0a}\u{4e0b}\u{6587}"),
        "pre-anim banner wording must no longer match"
    );
    assert!(!super::is_warn_flash("\u{2192} act mode"));
    assert!(!super::is_warn_flash("busy"));
}
