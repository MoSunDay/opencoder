Commit: (working-tree, pre-initial-commit)

# plan question 弹窗自定义行去掉重复的 ✎ 图标

## 背景
用户反馈：plan 模式 `question` 弹窗里，自定义输入行的占位符前面出现两个笔图标（`✎ ✎ custom answer…`）。

## 变更
### tui
- **`crates/tui/src/question_menu/view.rs`**：`custom_line` 空输入占位符原为 `"✎ custom answer…"`（view.rs:91），而函数末尾 `format!("✎ {body}")`（view.rs:104）又会统一前置一个 `✎`，导致空态渲染出两支笔。将占位符改为 `"custom answer…"`，笔图标只由 `format!` 添加一次；有自定义文本时的渲染不变（仍是一支笔 + 用户文本）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 自定义行恰好一支笔（空态/聚焦态） | `custom_row_shows_exactly_one_pen_icon` | crates/tui/src/question_menu/view.rs |

- 全量回归：`cargo test --workspace` → 全绿（0 failed）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：crates/tui/src/question_menu/view.rs 212 ≤ 800

## Impact Surface
- 用户可感知：plan question 弹窗自定义占位行从 `✎ ✎ custom answer…` 变为 `✎ custom answer…`。
- 不影响：键位状态机、作答/跳过语义、session/store/web/CLI 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [plan-question-tool](../2026-08-14/plan-question-tool.md)
