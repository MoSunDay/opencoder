//! Direct assertions on the submit-arm flash wiring: `queue_submit_flash` /
//! `steer_submit_flash` must set `mode_flash` exactly when the admitter
//! hand-off fails (actor gone / channel saturated) and leave it untouched on
//! success. The underlying failure bool and the flash constants are covered
//! in `queue_admitter` / `steer_admit`; these tests pin the `if` in the
//! helpers themselves (review flaw A).

use tokio::sync::mpsc;

use crate::app_helpers::{queue_submit_flash, steer_submit_flash};
use crate::queue_admitter::{AdmitReq, AdmitUiState, QUEUE_SUBMIT_FAILED_FLASH};
use crate::steer_admit::STEER_SUBMIT_FAILED_FLASH;

const TICK: u32 = 7;

#[test]
fn steer_submit_flash_failure_sets_mode_flash() {
    let (tx, rx) = mpsc::channel::<AdmitReq>(1);
    drop(rx); // dead actor → hand-off fails
    let mut st = AdmitUiState::default();
    let mut steer_items = vec![];
    let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
    let mut mode_flash: Option<(String, u32)> = None;

    steer_submit_flash(
        &tx,
        &mut st,
        &mut steer_items,
        &mut pending_images,
        "s",
        "stop",
        TICK,
        &mut mode_flash,
    );

    assert_eq!(
        mode_flash,
        Some((STEER_SUBMIT_FAILED_FLASH.to_string(), TICK)),
        "failed steer hand-off must flash"
    );
    assert!(steer_items.is_empty(), "temp row rolled back");
    assert_eq!(
        pending_images,
        vec![("img.png".to_string(), "alt".to_string())],
        "images restored to the composer"
    );
}

#[test]
fn queue_submit_flash_failure_sets_mode_flash() {
    let (tx, rx) = mpsc::channel::<AdmitReq>(1);
    drop(rx);
    let mut st = AdmitUiState::default();
    let mut queue_items = vec![];
    let mut pending_images = vec![("img.png".to_string(), "alt".to_string())];
    let mut mode_flash: Option<(String, u32)> = None;

    queue_submit_flash(
        "next task",
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending_images,
        "s",
        TICK,
        &mut mode_flash,
    );

    assert_eq!(
        mode_flash,
        Some((QUEUE_SUBMIT_FAILED_FLASH.to_string(), TICK)),
        "failed queue hand-off must flash"
    );
    assert!(queue_items.is_empty(), "temp row rolled back");
    assert_eq!(
        pending_images,
        vec![("img.png".to_string(), "alt".to_string())],
        "images restored to the composer"
    );
}

#[test]
fn submit_flash_helpers_keep_mode_flash_clear_on_success() {
    let (tx, mut rx) = mpsc::channel::<AdmitReq>(4);
    let mut mode_flash: Option<(String, u32)> = None;

    let mut st = AdmitUiState::default();
    let mut steer_items = vec![];
    let mut pending_images = vec![];
    steer_submit_flash(
        &tx,
        &mut st,
        &mut steer_items,
        &mut pending_images,
        "s",
        "stop",
        TICK,
        &mut mode_flash,
    );

    let mut st = AdmitUiState::default();
    let mut queue_items = vec![];
    queue_submit_flash(
        "next task",
        &tx,
        &mut st,
        &mut queue_items,
        &mut pending_images,
        "s",
        TICK,
        &mut mode_flash,
    );

    assert!(mode_flash.is_none(), "successful hand-offs never flash");
    assert_eq!(steer_items.len() + queue_items.len(), 2);
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_ok());
}
