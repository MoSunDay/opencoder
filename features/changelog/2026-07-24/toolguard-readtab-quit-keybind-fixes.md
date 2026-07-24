# Fix: tool-guard threshold, read/grep tool hardening, graceful Ctrl+D quit, keybind cleanup, stats provider

**Date:** 2026-07-24
**Scope:** `opencoder-core`, `opencoder-session`, `opencoder-tui`, scripts

## Background

A set of small, independent polish/fix changes accumulated in the working tree:

1. **Tool-failure guard too eager (P1):** The default consecutive-failure threshold
   was 3, aborting the agent loop prematurely on transient flaky-tool runs. Bumped
   to 5 to give the model more recovery room while still breaking doom-loops.
2. **Read tool tabs misalign TUI gutter (P1):** Tabs (`\t`) in files render as 0
   columns in ratatui but expand to the next 8-column stop in a real terminal,
   pushing file content past the line-number gutter (observed on macOS). The
   `ReadTool` now expands tabs to spaces before emitting numbered lines.
3. **Ctrl+D quit froze without feedback (P2):** On `KeyAction::Quit` the loop
   `break`-ed immediately, so the "shutting down…" status never rendered before
   the worker-shutdown wait. Now it renders one frame with a "shutting down…"
   status before breaking.
4. **Dead Ctrl+N / Ctrl+P history keybinds (P3):** Inside the multiline editor
   mode, `Ctrl+N`/`Ctrl+P` were wired to history navigation but were
   unused/removed earlier; the orphaned handlers and help-text lines were cleaned
   up.
5. **Stats script provider mapping (P3):** `parse_model` mapped `glm-5.2` to a
   `glm-5.2` provider, but the correct provider id is `zhipuai-coding-plan`.
   Fixed the mapping and its test.
6. **Grep tool hung on symlink cycles (P1):** The `GrepTool` directory walk
   followed symlinks but had no cycle guard, so a self-referencing symlink
   (`loop -> .`) recursed until the 50 000-file / 1000-result cap. The walk now
   tracks the canonical (real) path of each visited directory in a `HashSet` and
   skips already-seen canonical dirs, breaking the cycle while still following
   legitimate symlinked directories/files.

## Changes

| Change | File | Detail |
|--------|------|--------|
| Tool-guard threshold 3→5 | `crates/core/src/tool_guard_config.rs` | `default_tool_failure_threshold()` returns `5` (was `3`) |
| Threshold doc/comment | `crates/core/src/config.rs` | `tool_guard` doc comment updated to "5 consecutive failures" |
| Threshold memory doc | `agents/session/index.md` | `max_consecutive_failures` documented value updated 3→5 |
| Read-tool tab expansion | `crates/session/src/tools/read.rs` | new pure `expand_tabs()` advances column to next 8-col stop; applied per line in output (`{:>5}: {}`) |
| Grep symlink-cycle guard | `crates/session/src/tools/grep.rs` | `walk()` now takes `&mut HashSet<PathBuf>`; canonicalizes each dir via `canonicalize()` and skips already-seen real paths |
| Grep test coverage | `crates/session/tests/tools_contract.rs` | 4 new `#[cfg(unix)]` tests: symlink cycle, symlinked dir, symlinked file, glob self-ref |
| Ctrl+D graceful quit | `crates/tui/src/app.rs` | `KeyAction::Quit` sets `quitting=true` + "shutting down…" status + dirty/render flags; loop renders one frame then `break`s at top-of-loop `if quitting { break; }` |
| History keybind cleanup | `crates/tui/src/key_handler.rs` | removed dead `Ctrl+N`/`Ctrl+P` history handlers in multiline mode |
| Keybind help text | `crates/tui/src/keybind.rs` | removed `Ctrl+N`/`Ctrl+P` and duplicate `Home/End` lines |
| Stats provider mapping | `scripts/opencoder-to-opencode-stats.py` | `parse_model`: `glm-5.2` model id → `zhipuai-coding-plan` provider |
| Stats test | `scripts/test-stats-sync.py` | `test_parse_model` expects `zhipuai-coding-plan`; added bare-provider check |

## Tests

| Test | File | Asserts |
|------|------|---------|
| `expand_leading_tab` | `crates/session/src/tools/read.rs` | leading tab → 8 spaces |
| `expand_mid_line_tab_advances_to_next_stop` | `crates/session/src/tools/read.rs` | `ab\tcd` → `ab      cd` (6 spaces) |
| `expand_consecutive_tabs` | `crates/session/src/tools/read.rs` | two tabs → 16 spaces |
| `no_tab_returns_unchanged` | `crates/session/src/tools/read.rs` | no-op for tab-free input |
| `tab_at_eighth_column_adds_eight_spaces` | `crates/session/src/tools/read.rs` | advances to next stop at col 16 |
| `empty_string_unchanged` | `crates/session/src/tools/read.rs` | empty input no-op |
| `grep_follows_symlink_but_breaks_cycle` | `crates/session/tests/tools_contract.rs` | self-ref `loop -> .` yields exactly one match (cycle broken) |
| `grep_includes_symlinked_directory` | `crates/session/tests/tools_contract.rs` | symlinked dir outside root is searched |
| `grep_includes_symlinked_file` | `crates/session/tests/tools_contract.rs` | symlinked file outside root is read |
| `glob_survives_self_referencing_symlink` | `crates/session/tests/tools_contract.rs` | glob `**` does not hang on self-ref symlink |
| `threshold_stops_after_five_consecutive_failures` | `crates/session/tests/tool_failure_guard.rs` | loop aborts after 5 consecutive failures; 6th script unconsumed |
| `emits_error_event_on_threshold` | `crates/session/tests/tool_failure_guard.rs` | error event emitted at threshold (now 5) |
| `success_between_failures_resets_counter` | `crates/session/tests/tool_failure_guard.rs` | success resets counter; needs 5 consecutive to trip |
| `test_parse_model` | `scripts/test-stats-sync.py` | `glm-5.2` → provider `zhipuai-coding-plan` |

- 全量回归：`cargo test --workspace` → 全绿 (0 failures)
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 编译干净
- 行数：`crates/tui/src/app.rs` 800 ≤ 800；`crates/core/src/config.rs` 779 ≤ 800；`scripts/opencoder-to-opencode-stats.py` 399 ≤ 400；其余改动文件均 ≤ 400/800

## Impact Surface

- 用户可感知：agent 在连续工具失败时更宽容（5 次而非 3 次才中止）；带 Tab 的文件在 TUI 中行号沟槽对齐；grep 遇自引用 symlink 不再卡死且仍跟随合法软链；Ctrl+D 退出时显示 "shutting down…" 反馈；移除未用的 Ctrl+N/P 历史键绑定。
- 不影响：CLI/Web/Store/session drain 边界（仅默认常量、read/grep 工具实现与 TUI 渲染循环）。
- 仅 stats 脚本（非运行时）受 provider 映射影响。

## Related Docs

- [agents/session](../../agents/session/index.md)
- [既有 changelog: tool-failure-surface-fixes](./tool-failure-surface-fixes.md)
