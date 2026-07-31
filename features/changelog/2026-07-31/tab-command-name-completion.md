# tab-command-name-completion

## Summary

Tab in the `:` command popup now fills the composer with the selected
command's canonical name (e.g. `/plan`) and closes the popup, replacing the
former behaviour of queuing a control command behind a running turn. Tab is
now a predictable "accept-as-text" completion: the user can edit the filled
text and submit it normally from the composer. The dead `CommandOutcome::Queue`
variant and its (unreachable) match arm — the only producer of the old
queue-on-Tab path — were removed, along with the now-unused `queue_items`
parameter of `dispatch_command`.

## Changes

### `crates/tui/src/command.rs`
- Added `CommandOutcome::FillInput(String)` variant: Tab fills the input and
  closes the popup.
- Added `CommandMenu::selected_name() -> Option<&'static str>` returning the
  canonical command text of the highlighted row.
- Rewrote the Tab branch to emit `FillInput(selected_name())` for every command
  row (control, local, and slash commands alike), then close the popup.
- Removed the dead `CommandOutcome::Queue(String)` variant (no construction
  site remained after the Tab rewrite).

### `crates/tui/src/app_loop.rs`
- Removed the `CommandOutcome::Queue(s) => { ... }` match arm in
  `dispatch_command` (unreachable once the Tab branch stopped emitting `Queue`).
- Removed the now-unused `queue_items: &mut Vec<(i64, String)>` parameter from
  `dispatch_command` and updated its sole call site.
- The `CommandOutcome::FillInput(s)` arm clears the composer, writes the
  command name, resets the cursor, and returns `LoopFlow::Redraw`.

### `crates/tui/src/app.rs`
- Updated the `dispatch_command(...)` call site to drop the `queue_items`
  argument.

## Test checklist

### Unit tests — Tab completion (`cargo test -p opencoder-tui --lib command`)
- `tab_fills_input_with_command_name` — asserts `FillInput("/plan")` + popup
  closed.
- `tab_on_non_control_command_fills_input` — asserts `FillInput("/task")`.
- `tab_on_local_command_fills_input` — asserts `FillInput("/ps")`.
- `tab_on_install_tools_fills_input` — asserts `FillInput("/install_tools")`.

## Impact surface
- User-facing: Tab in the `:` popup now completes to editable text instead of
  silently queuing a control command. Enter semantics are unchanged (separate
  branch), so immediate dispatch is unaffected.
- Dead-code removal only on the TUI command path; `CommandOutcome` is now
  `{ Idle, Dispatch, FillInput }`.
- No storage / LLM / runner changes; `Store` and `ChatStream` seams untouched.
