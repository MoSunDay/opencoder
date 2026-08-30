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

mod cancel_keep_pending;
mod cli_outcome_tests;
mod envs_outcome_tests;
mod mcp_outcome_tests;
mod model_outcome_tests;
mod skill_outcome_tests;

mod done_error_mirror_tests;

mod plan_chip_consume_tests;

mod display_title_tests;
mod tok_cost_idle_refresh_tests;

#[cfg(test)]
#[path = "../app_loop_plan_edit_tests.rs"]
mod plan_edit_tests;

#[cfg(test)]
#[path = "../app_loop_session_only_tests.rs"]
mod session_only_tests;

#[cfg(test)]
#[path = "../app_loop_ap_outcome_tests.rs"]
mod ap_outcome_tests;

mod image_paste_tests;

#[cfg(test)]
#[path = "../app_loop_dispatch_cmd_tests/mod.rs"]
mod dispatch_cmd_tests;

#[cfg(test)]
#[path = "../app_loop_slash_action_tests.rs"]
mod slash_action_tests;

#[cfg(test)]
mod switch_gate_tests;
