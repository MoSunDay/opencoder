# install-tools-tui-command

## Summary

Added a `/install_tools` TUI slash command that detects the optional tool
dependencies (tmux + chromium) and, when any are missing, suspends the TUI,
runs the embedded `install-skills-dep.sh` with inherited stdio (so the user
can type a sudo password), then resumes the screen and re-seeds the
dep-gated `ssh-pty` / `chrome-headless` skills. Also refactored the CLI
exit-tips probe to reuse the shared dependency check.

## Changes

### `crates/core/src/tool_deps.rs` (NEW, 111 lines)
- `ToolDepStatus { tmux, chrome, sentinel }` value type.
- `check_tool_deps()` — probes `tmux -V`, finds a chromium binary (mirrors
  `find_chrome()` from `exit_tips`), and checks the skills-deps sentinel.
- `all_installed(&status)` helper.
- 2 unit tests (truth table + default-not-installed).
- Re-exported from `crates/core/src/lib.rs` (`tool_deps` mod + `pub use`).

### `crates/tui/src/install_tools.rs` (NEW, 204 lines)
- `run(terminal, &mut chat)` orchestrator: detect → if all installed, push a
  green "nothing to do" marker and return; else write the script, suspend the
  screen, run the installer with inherited stdio, resume + clear, re-seed
  dep-gated skills, then push formatted result markers.
- `install_script_path()` — derives `~/.opencoder/install-skills-dep.sh` from
  `skills_dir().parent()` (avoids a `dirs` dep in the tui crate).
- `suspend_and_run()` — `terminal::suspend_screen()`, run `Command::status()`
  with inherited stdio, return exit code.
- `format_result(exit_code, &status)` — pure: builds 2-3 styled marker lines.
- 4 unit tests on `format_result` (success/all-installed, success/still-missing,
  failure-still-shows-status, failure-has-no-success-tail).

### `crates/tui/src/terminal.rs`
- `suspend_screen()` / `resume_screen()` (each `-> anyhow::Result<()>`):
  teardown/rebuild of raw mode + alternate screen using the same crossterm
  sequences as `TerminalGuard::enter()` / `write_restore()`, so a foreground
  child process gets a clean TTY and the TUI is fully repainted on resume.

### `crates/tui/src/command.rs`
- `("/install_tools", ...)` entry in `COMMANDS`.
- `SlashAction::InstallTools` variant.
- `parse()` arm (`install_tools`), `dispatch()` arm (`/install_tools`).
- 4 tests: parse (slash/trim), dispatch, Enter-dispatches, Tab-fills-input.

### `crates/tui/src/app_loop.rs` + `app.rs`
- `LoopFlow::InstallTools` variant (between `Redraw` and `Quit`).
- `handle_command_key` Enter → `CommandOutcome::Dispatch(InstallTools)` →
  `LoopFlow::InstallTools` (returns early).
- 3 exhaustive match arms in `app.rs`: model_menu (`{}`), command_picker
  (`install_tools::run(terminal, &mut chat)` + redraw), fold (`dirty = true`).

### `crates/cli/src/exit_tips.rs`
- Replaced private `tmux_available()` / `deps_sentinel_exists()` with the
  shared `opencoder_core::check_tool_deps()`; dropped the now-unused
  `use std::process::Command` and dead `use super::*` test import.


## Test checklist

### Unit tests
- `cargo test -p opencoder-core --lib tool_deps`
  - `default_status_is_not_installed`
  - `all_installed_truth_table`
- `cargo test -p opencoder-tui --lib install_tools`
  - `format_result_success_all_installed`
  - `format_result_success_but_still_missing`
  - `format_result_failure_even_if_deps_present`
  - `format_result_failure_has_no_tail`
- `cargo test -p opencoder-tui --lib command`
  - `parse_install_tools`
  - `dispatch_install_tools`
  - `enter_on_install_tools_dispatches`
- `cargo test -p opencoder-cli --lib exit_tips`
  - `sentinel_check_does_not_panic`

### Full regression
- `cargo test --workspace` → 1434 passed; 0 failed; 0 ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`
  → zero warnings.

## Impact surface
- New user-facing command: `/install_tools` (also Tab/Enter-discoverable in the
  `:` command popup). Triggers an external process with inherited stdio only
  when deps are missing; otherwise a no-op marker.
- No storage / LLM / runner changes; `Store` and `ChatStream` seams untouched.
