# update-subcommand

## Summary

Added `opencoder update` — a parameterless subcommand that runs a **built-in
prompt** through the existing headless agent (`run_headless`), delegating the
entire self-update workflow to the agent itself: clone the latest `main` from
the public repo, rebuild, and atomically swap the `opencoder` binary on `PATH`
(handling the ETXTBSY / busy case). No git/cargo calls are hardcoded in Rust;
the update is performed by the LLM agent executing the embedded prompt.

## Changes

### `crates/cli/src/update.rs` (NEW)
- `UPDATE_PROMPT` constant: the built-in update instructions (Chinese, verbatim
  as specified) — clone latest main, build, replace the PATH binary, handle the
  busy case, ensure the new logic takes effect.
- `update_run(cli)`: thin handler that calls `run::run_headless(cli,
  UPDATE_PROMPT.to_string())`. Config loading, LLM-client construction, event
  rendering, Ctrl-C cancellation, and async title generation are all inherited
  from `run_headless` with zero duplication.

### `crates/cli/src/lib.rs`
- Added `pub mod update;` to the module declarations.
- Added `Update` variant to the `Command` enum (parameterless). Documented as a
  self-update that runs the built-in prompt headlessly.

### `src/main.rs`
- Added dispatch arm `Some(Command::Update) =>
  opencoder_cli::update::update_run(&cli).await` before the `None` fallback.
- `is_tui` judgment unchanged: `Update` is not `Tui`/`Ts`/bare-empty, so it
  correctly takes the headless stdout-log path. The built-in prompt is passed
  directly (not via `require()`), so there is no "empty prompt" guard concern.

## 测试覆盖

`cargo test -p opencoder-cli --test cli_parse` -> **25 passed; 0 failed; 0 ignored** (this session; full `cargo test -p opencoder-cli` = 64 passed across binaries). `cargo build -p opencoder` clean. `opencoder update --help` shows the
new subcommand + doc text.

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `opencode update` 解析为 `Command::Update` | `update_subcommand` | `crates/cli/tests/cli_parse.rs` |

Manual `opencoder update` (requires an API key) verified to admit the built-in
prompt and begin the agent-driven update.
