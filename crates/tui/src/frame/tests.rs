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
fn warn_flash_hue_covers_sandbox_plan_and_clear_guard() {
    assert!(super::is_warn_flash("\u{2192} sandbox mode"));
    assert!(super::is_warn_flash("\u{2192} plan mode"));
    assert!(super::is_warn_flash("\u{2192} clear 5s \u{540e}\u{6e05}\u{7a7a}\u{4e0a}\u{4e0b}\u{6587}"));
    assert!(!super::is_warn_flash("\u{2192} act mode"));
    assert!(!super::is_warn_flash("busy"));
}
