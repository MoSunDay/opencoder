Commit: (working-tree, pre-initial-commit)

# refactor(tui): app_tests.rs 模块拆分 + changelog 全量回归数对齐

## 背景

Review 发现两个问题：
1. **changelog 全量回归数不一致**：5 个 changelog 的 `cargo test --workspace` 总数
   引用了旧快照（1376 / 1380 / 1382），而实际工作区已增长至 1396 passed。
2. **`app_tests.rs` 超过 800 行迭代限制**：单文件 1581 行（HEAD 基线 1465 + WIP 116），
   违反仓库规则"迭代中的文件不得超过 800 行"。

此外，stash 中有尚未提交的测试文件（skill 组合内容、apply_force_redraw、
apply_skill_tokens 组合内容），需一并落盘。

## 变更

### app_tests.rs → app_tests/ 目录拆分
- **`crates/tui/src/app_tests.rs`**（删除）：1581 行单体测试文件移除。
- **`crates/tui/src/app_tests/mod.rs`**（171 行）：共享 helper（`key`/`run_handle`/
  `run_handle_disabled`/`run_handle_subagent`/`run_handle_menu`）标记 `pub(super)`，
  所有 import 标记 `pub(super) use`，声明 `mod key_tests; mod skill_tests;`。
- **`crates/tui/src/app_tests/key_tests.rs`**（719 行）：enter/tab/ctrl_u/shift_tab/
  ctrl_a/e/ctrl_w/cursor/quit 全部键处理与编辑测试。
- **`crates/tui/src/app_tests/skill_tests.rs`**（453 行）：sys_tokens/dollar/skill_menu/
  flash/skill_display/worker 全部 skill 相关测试。
- **`crates/tui/src/app.rs`**：`#[path = "app_tests.rs"] mod tests;` →
  `#[path = "app_tests/mod.rs"] mod tests;`。
- **Bug 修复**：恢复 `flash_visible_within_window` 与 `start_turn_reports_false`
  被误删的 `#[test]` 属性。

### changelog 全量回归数对齐（1376/1380/1382 → 1396）
- **`features/changelog/2026-07-30/bash-exit-code-and-timeout-tweak.md`**：1376 → 1396
- **`features/changelog/2026-07-30/prompt-working-dir-parenthetical-and-plan-marker.md`**：1391 → 1396
- **`features/changelog/2026-07-30/tui-ctrl-l-force-redraw-and-control-char-sanitize.md`**：1391 → 1396
- **`features/changelog/2026-07-30/tui-ps-stop-display-only-commands.md`**：1391 → 1396
- **`features/changelog/2026-07-30/tui-theme-modernization.md`**：1391 → 1396；TUI lib 646 → 651

### 补提 WIP 测试（从 stash 恢复）
- **`crates/core/src/skill.rs`**：5 个 `extract_skill_tokens` 组合内容测试（+48 行）。
- **`crates/tui/src/app_helpers_tests/mod.rs`**：`apply_force_redraw` 的 needs_clear / no-op
  两个单元测试（+74 行）。
- **`crates/tui/src/app_helpers_tests/skill_apply.rs`**（249 行，新建）：3 个
  `apply_skill_tokens_combined_content` 测试（token 在末尾 / 文中 / 多 token 间隔）。
- **`agents/tui/index.md`**：repair-on-touch，更新 skill_persist 条目。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| extract_tokens token at end after text | `extract_tokens_token_at_end_after_text` | `crates/core/src/skill.rs` |
| extract_tokens realistic combined input | `extract_tokens_realistic_combined_input` | 同上 |
| extract_tokens curly brace in other content preserved | `extract_tokens_curly_brace_in_other_content_preserved` | 同上 |
| apply_force_redraw clears terminal | `apply_force_redraw_clears_terminal_and_sets_flags_when_needs_clear` | `crates/tui/src/app_helpers_tests/mod.rs` |
| apply_force_redraw is a no-op | `apply_force_redraw_is_a_noop_when_needs_clear_false` | 同上 |
| apply_skill_tokens token at end | `apply_skill_tokens_combined_content_token_at_end` | `crates/tui/src/app_helpers_tests/skill_apply.rs` |
| app_tests split compiles & all tests pass | 全部 key_tests + skill_tests | `crates/tui/src/app_tests/` |

- 全量回归：`cargo test --workspace` → **1396 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → **0 warnings**
- build：`cargo build --workspace` → **Finished, 0 errors**
- 行数：`app_tests/mod.rs` 171 ≤ 800 ✓ ｜ `key_tests.rs` 719 ≤ 800 ✓ ｜
  `skill_tests.rs` 453 ≤ 800 ✓ ｜ `skill_apply.rs` 249 ≤ 400 ✓

## Impact Surface

- **零行为变更**：纯测试拆分 + changelog 文档对齐，不改动任何运行时代码逻辑。
- 唯一源码改动是 `app.rs` 的 `#[path]` 属性（指向新目录），编译后行为不变。
- 恢复 2 个误删的 `#[test]` 属性使它们重新被 test harness 执行。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
