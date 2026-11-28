//! Tests for `app_loop` helpers — extracted to keep `app_loop.rs` under the
//! 800-line cap. Compiled as `#[cfg(test)] mod tests` via `#[path]`.

use super::*;
use crate::chat::ChatView;

// ----- Shared test infrastructure (used by submodules) -----

/// Single process-global lock serializing every test that *reads* the global
/// config / `home_dir()` while a sibling could conceivably touch it. The
/// former env-mutating tests now use thread-local `scoped_config_home`
/// instead (no `std::env::set_var`), so this lock is retained only as a
/// belt-and-suspenders serializer — it is no longer load-bearing for safety.
pub(crate) static HOME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acknowledge worker commands while preserving them for the test's original
/// assertions. Failure-path tests drive their channels manually instead.
pub(crate) fn auto_ack_commands(
    mut commands: tokio::sync::mpsc::Receiver<crate::worker::UiCmd>,
) -> tokio::sync::mpsc::Receiver<crate::worker::UiCmd> {
    let (forward_tx, forward_rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(command) = commands.recv().await {
            match &command {
                crate::worker::UiCmd::ReloadConfig { ack: Some(ack), .. }
                | crate::worker::UiCmd::ApplySession { ack, .. }
                | crate::worker::UiCmd::ReloadEnv { ack, .. } => {
                    let _ = ack.send(Ok(()));
                }
                _ => {}
            }
            if forward_tx.send(command).await.is_err() {
                break;
            }
        }
    });
    forward_rx
}

mod cancel_keep_pending;
mod cli_outcome_tests;
mod mcp_outcome_tests;
mod model_outcome_tests;
mod skill_outcome_tests;

mod config_busy_gate_tests;
mod done_error_mirror_tests;
mod env_model_failure_tests;
mod env_model_tests;
mod model_deferred_switch_tests;

mod compact_done_rekick;
mod stale_turn_done_tests;

mod quit_unblock_tests;
mod steer_remote_interrupt_tests;

mod plan_chip_consume_tests;

mod display_title_tests;
mod render_gate_fuzz;
mod sidecar_display_tests;
mod sidecar_usage_tests;
mod tok_cost_idle_refresh_tests;
mod transcript_reset_echo_tests;
mod transcript_reset_offloop_tests;

#[cfg(test)]
#[path = "../app_loop_plan_edit_tests.rs"]
mod plan_edit_tests;

#[cfg(test)]
#[path = "../app_loop_session_only_tests.rs"]
mod session_only_tests;

mod image_paste_tests;

#[cfg(test)]
#[path = "../app_loop_dispatch_cmd_tests/mod.rs"]
mod dispatch_cmd_tests;

#[cfg(test)]
#[path = "../app_loop_slash_action_tests.rs"]
mod slash_action_tests;

#[cfg(test)]
mod switch_gate_tests;
