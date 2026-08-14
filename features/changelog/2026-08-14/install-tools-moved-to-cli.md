# install-tools-moved-to-cli

## Summary

Removed the TUI `/install_tools` slash command and replaced it with a
`opencode install-tools` CLI subcommand. The execution logic is unchanged
(probe → run embedded `install-skills-dep.sh` with inherited stdio for the
sudo password → re-seed dep-gated skills → report); only the surface moved:
plain stdout lines instead of TUI chat markers, and no TUI screen
suspend/resume needed since the CLI owns the terminal already. Non-zero
installer exit codes now propagate as the process exit status.

Also: status-bar `thr`/`ctx` label colour split (only the meter + percent
value follow the threshold colour; labels use a new soft light-blue
`theme::light_blue()` — `LightBlue` on dark, `Blue` on light).

## Changes

### `crates/cli/src/install_tools.rs` (NEW, 147 lines)
- `install_tools_run() -> anyhow::Result<i32>` — orchestrator ported from the
  former TUI module; returns the installer exit code for propagation.
- `install_script_path()` — unchanged (`skills_dir().parent()` derivation).
- `format_result()` — same three-line/two-line report, plain `String` lines.
- 4 unit tests (ported: success-all-installed, still-missing, non-zero exit
  variants).

### `crates/cli/src/lib.rs`
- `pub mod install_tools;`
- `Command::InstallTools` subcommand variant (kebab-case `install-tools`).

### `src/main.rs`
- Dispatch arm: runs `install_tools_run()`, `std::process::exit(code)` on
  non-zero.

### Removals (TUI)
- `crates/tui/src/install_tools.rs` deleted.
- `crates/tui/src/command.rs`: `/install_tools` COMMANDS entry,
  `SlashAction::InstallTools`, parse/dispatch arms, 4 tests.
- `crates/tui/src/app_loop.rs`: `LoopFlow::InstallTools` variant.
- `crates/tui/src/app.rs`: 3 match arms calling `install_tools::run`.
- `crates/tui/src/app_loop_actions.rs`: dispatch arm + doc mention.
- `crates/tui/src/terminal.rs`: dead `suspend_screen`/`resume_screen`
  (only caller was the TUI suspend flow).

### Misc
- `crates/cli/src/exit_tips.rs`: setup hint now `opencode install-tools`.
- `crates/tui/src/render_status.rs` + `render_tests/status_ctx.rs`: `thr`
  label and `ctx (used/limit)` use `theme::light_blue()`; meter + percent
  keep the semantic threshold colour.
- `crates/tui/src/theme.rs`: `LIGHT_BLUE` const + `Palette::light_blue`
  slot (`LightBlue` dark / `Blue` light) + `light_blue()` resolver;
  `palette_dark_matches_constants` extended.

## Tests

| File | Tests |
|---|---|
| `crates/cli/src/install_tools.rs` | 4 unit tests (format_result variants) |
| `crates/cli/tests/cli_parse.rs` | `install_tools_subcommand` |
| `crates/tui/src/render_tests/status_ctx.rs` | `status_bar_colors_split_between_meter_and_labels` |

Full workspace regression: `cargo test --workspace` — 2541 passed, 0 failed.
