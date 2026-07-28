Commit: (working-tree, pre-initial-commit)

# refactor(tui/core): 模块拆分控制行数 + 修复迁移编译断点

## 背景

多个文件逼近/超过 800 行迭代上限。按职责边界拆分子模块。拆分后部分调用点未同步更新（merge 函数未加 `merge::` 前缀、autopilot merge 被 TEMP 注释、测试 import 路径失效），导致编译/clippy 断点。本次一并修复。

## 变更

### TUI key_handler 拆分
- `crates/tui/src/key_handler.rs`：-546 行，纯函数与 `KeyAction` 留在本体
- `crates/tui/src/key_handler_tests.rs`（新 315 行）：`handle_key` 单元测试
- `crates/tui/src/key_handler_plan_edit_tests.rs`（新 345 行）：plan-edit 模式按键测试

### TUI chat 拆分
- `crates/tui/src/chat.rs`：`summarize` / `short` / `block_text` 迁出
- `crates/tui/src/chat_helpers.rs`（新 42 行）：三个纯函数

### TUI app 相关拆分
- `crates/tui/src/app_bootstrap.rs`（新 118 行）：从 `app.rs` 抽离启动/恢复初始化
- `crates/tui/src/subagent_input.rs`（新 227 行）：subagent steer 录入 helper
- `crates/tui/src/skill_display.rs`（新 25 行）：skill token 显示/触发

### Core config 模块化
- `crates/core/src/config.rs`：-222 行，拆为 `config/autopilot.rs` + `config/merge.rs`

### 迁移编译断点修复（本次提交补充）
- `crates/core/src/config.rs`：3 处调用点改用 `merge::` 前缀（`merge::merge_into` / `merge::has_editable_key` / `merge::merge_json`）
- `crates/core/src/config/merge.rs`：取消 `autopilot::merge` 的 TEMP-VERIFICATION 注释，恢复 autopilot 配置合并
- `crates/tui/src/app_tests.rs`：`resume_hint` import 路径改为 `crate::app_helpers`
- `crates/tui/src/chat_tests/mod.rs`：补充 `use crate::composer`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| key_handler 行为 | key_handler_tests 各项 | `crates/tui/src/key_handler_tests.rs` |
| plan-edit 按键 | 各项 | `crates/tui/src/key_handler_plan_edit_tests.rs` |
| 配置合并 | autopilot contract | `crates/core/tests/config_contract.rs` |

## 验证

全量回归跑通：

| 检查 | 结果 |
|------|------|
| `cargo test --workspace` | PASS — 全绿 |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |

## Impact Surface

- 纯结构重构，行为不变
- 不影响 CLI/Web/session/store 边界

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [agents/core](../../agents/core/index.md)
