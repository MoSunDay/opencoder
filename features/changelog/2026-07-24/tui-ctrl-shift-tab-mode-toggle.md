Commit: (working-tree, pre-initial-commit)

# fix(tui): 模式切换键 t+Tab 改为 Ctrl+Shift+Tab

## 背景
此前「不清空 context、不自动执行、仅切换 plan/act 模式」的快捷键是 `t`+`Tab`
和弦：在输入框输入恰好一个 `t` 再按 Tab。该和弦有两个问题：
1. **侵入输入语义**：用户想在 act 模式提交单个字母 `t` 时无法直接 Tab 提交，
   必须先清空再输入。
2. **会消费输入**：触发后 `input` 被清空、`cursor_idx` 归零，草稿丢失。

需求：换用 `Ctrl+Shift+Tab`，且**只切状态**——不清空输入框、不自动执行、不丢草稿。

## 变更
### 新增 Ctrl+Shift+Tab 检测（`crates/tui/src/key_handler.rs`）
- 在 `handle_key` 的 Alt+Tab 检测之后、CONTROL 修饰符分支之前，新增
  `Ctrl+Shift+Tab → SwitchAgentNoClear` 分支（key_handler.rs:80-90）。
- 放在 CONTROL 分支之前是必须的：否则 Tab/BackTab 被 CONTROL 分支的
  early-return 吞掉，永远到不了。
- 终端兼容性：`BackTab` + `CONTROL`（主流终端对 Ctrl+Shift+Tab 的编码）
  和 `Tab` + `CONTROL | SHIFT`（kitty keyboard protocol 全消歧模式）都识别。

### 移除 t+Tab 和弦（`crates/tui/src/key_handler.rs`）
- 删除 `KeyCode::Tab` arm 中的 `input.trim() == "t"` 和弦判断；
  Tab 现在纯粹是 submit(idle)/queue(running)，不再有特殊模式。

### SwitchAgentNoClear 不再清空输入（`crates/tui/src/key_handler.rs`）
- Ctrl+Shift+Tab 分支直接返回 `SwitchAgentNoClear`，不触碰 `input` /
  `cursor_idx`，草稿完整保留。

### 帮助文本更新（`crates/tui/src/keybind.rs`）
- `t+Tab ...` → `Ctrl+Shift+Tab  switch mode act <--> plan (keep context, no handoff reset)`

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+Shift+Tab 在 act 模式切到 plan，输入保留 | `ctrl_shift_tab_in_act_mode_switches_without_clear` | `tui/src/app_tests.rs`（改写） |
| Ctrl+Shift+Tab 在 plan 模式切到 act，输入保留 | `ctrl_shift_tab_in_plan_mode_switches_without_clear` | `tui/src/app_tests.rs`（改写） |
| 移除和弦后单个 `t`+Tab 正常 submit | `single_t_then_tab_submits_normally` | `tui/src/app_tests.rs`（改写） |

- 全量回归：`cargo test --workspace -- --test-threads=1` → 全绿
  （`sys_tokens_counts_system_prompt` 在并发下 flaky，为既有 temp_dir 并发问题，单独运行通过）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：key_handler.rs 505 / keybind.rs 24（均 ≤ 800）

## Impact Surface
- 仅影响 TUI 按键处理与帮助文本；SwitchAgentNoClear 的 dispatch 逻辑（`app.rs`）不变。
- 旧的 `t`+`Tab` 和弦不再可用；改用 `Ctrl+Shift+Tab`。
- `SwitchAgent`（Shift+Tab / Alt+Tab，含 handoff + 清空）行为不变。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
