# bash-timeout-restore-dedup

## Summary

Restored bash's foreground timeout mechanism (130 s) with automatic background
handoff. When a bash command exceeds `BASH_TIMEOUT_SECS`, it is moved to the
background supervisor instead of being killed — long-running builds keep going
and their output lands in `/tmp/opencoder_bg_{pid}.output`. Additionally,
consecutive bash timeouts are deduplicated: only the first timeout's full
message is shown to the model; subsequent ones reuse the first's content (same
PID, same output file) since the model reads the same file regardless.

## Changes

### `crates/session/src/tools/bash.rs`
- Added `BASH_TIMEOUT_SECS` (130 s; 1 s under `cfg(test)` for fast unit tests).
- Added `BASH_TIMEOUT_MARKER` (`"[bash-timeout:"`) for runner dedup detection.
- Replaced unconditional `child.wait().await` with
  `tokio::time::timeout(BASH_TIMEOUT_SECS, child.wait())` race.
- On timeout (Unix): captures accumulated output, calls `bg::handoff`, returns
  `is_error: false` marker message with pid + output file path
  (`/tmp/opencoder_bg_{pid}.output`) — the command keeps running in the
  background, so the tool-failure guard is not tripped.
- On timeout (non-Unix): falls back to `child.kill().await` and returns the
  same marker with "killed" (handoff is Unix-only since it relies on process
  groups).
- Updated imports: `Duration`, `bg::{handoff, output_path}`, test `kill_all`.

### `crates/session/src/runner/mod.rs`
- Added `dedup_consecutive_bash_timeouts()` pure function: replaces
  consecutive bash-timeout results with the first's content.
- State `bash_timeout_first: Option<String>` persists across turns in `run_loop`.
- Non-timeout bash result resets the streak; non-bash tools do not.
- 4 unit tests covering: first-stored/subsequent-replaced, reset on non-timeout
  bash, no-reset on non-bash tool, cross-batch persistence.

### `crates/session/src/tools/bg.rs`
- Updated module doc: `handoff` is now actively called on bash timeout.

### `crates/session/src/runner/execute.rs`
- Updated comments: bash exemption rationale now references `BASH_TIMEOUT_SECS`.

### `crates/session/tests/tools_contract.rs`
- Updated `bash_tool_runs_long_command_without_handoff` comment.

## Test checklist

### Unit tests (`cargo test -p opencoder-session --lib`)
- `bash_short_command_completes_normally` — echo completes under 1 s test timeout
- `bash_timeout_triggers_handoff` — sleep 3 exceeds 1 s, marker + pid + path present
- `bash_registers_while_running_unregisters_after` — sleep 0.5 completes normally
- `bash_normal_completion`, `bash_failure_appends_exit_code` — unchanged
- `parameters_schema_hides_timeout_from_model` — unchanged
- `dedup_tests::first_timeout_stored_subsequent_replaced`
- `dedup_tests::non_timeout_bash_resets_count`
- `dedup_tests::non_bash_tool_does_not_reset_count`
- `dedup_tests::first_persists_across_batches`

### Integration tests (`cargo test -p opencoder-session --test '*'`)
- All existing integration tests pass unchanged (non-cfg(test) build uses 130 s
  timeout, so `sleep 2`/`sleep 5`/`sleep 30` commands complete or are killed by
  `/stop` before the timeout fires).

### Static analysis
- `cargo clippy -p opencoder-session` — zero warnings (`handoff` no longer dead code).
