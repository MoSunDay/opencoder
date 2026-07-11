Commit: (working-tree, pre-initial-commit)

# `/` 与 `/model` 弹窗 UX — 锚定输入框上方 + Enter 确认推进下一项

## 背景
用户反馈两点：
1. 输入 `/` 弹出的命令面板、以及 `/model` 配置弹窗，都是屏幕居中的大模态，不够自然，希望像 IDE 自动补全那样**贴在输入框上方**。
2. 在 `/model` 配置里，回车在文本字段上是空操作，必须靠 Tab/Shift+Tab 切换字段；用户希望**回车即确认当前值并跳到下一项**，形成连续填写流。

## 变更
### 2a. 弹窗锚定 composer 上方（下拉浮层）
- **`crates/tui/src/command.rs`**：`render_command_popup` 新增 `composer_top: u16` 参数，底边（+1 行查询页脚）紧贴 composer 顶边，高度受上方可用行夹取；删除不再使用的 `centered()` 辅助。
- **`crates/tui/src/model_menu/view.rs`**：`render_model_popup` 同样新增 `composer_top`，底边贴 composer 顶部，高度 `min(16, composer_top)`。
- **`crates/tui/src/render.rs`**：把已算出的 `composer_area.y` 传给两个弹窗渲染函数。

### 2b. `/model` Enter = 确认 + 推进（↑/↓ 行为不变）
- **`crates/tui/src/model_menu/state.rs`**：`Enter` 分支改为——除 `Save`（提交）/`Cancel`（取消）外，其余字段（Model/BaseUrl/ApiKey/Reasoning/Threshold）一律 `m.focus = m.focus.next()`，即确认当前值并推进到下一项。Reasoning 不再被回车 toggle。`↑/↓` 保持原语义（文本字段导航、Reasoning/Threshold 改值），`←/→/Space` 保留为 Reasoning 循环——按用户选择「保留 ↑/↓ 改值」。
- **`crates/tui/src/model_menu/view.rs`**：标题提示更新为 `↑/↓ field, Enter=confirm/next, [Save] commits, Esc cancel`；各字段聚焦提示补 `Enter=next`。

最终交互流：填一项 → Enter → 下一项 → … → Enter on `[Save]` 提交。`↑/↓` 在输入框间跳转、在 Reasoning/Threshold 上调值。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Enter 在文本字段确认并推进 | `enter_on_text_field_advances_to_next` | `crates/tui/src/model_menu/mod.rs` |
| Enter 在 Reasoning 推进但不改值 | `enter_on_reasoning_advances_without_toggling` | `crates/tui/src/model_menu/mod.rs` |
| 连续 Enter 走完字段序到 Save | `enter_chains_through_fields_to_save` | `crates/tui/src/model_menu/mod.rs` |
| Enter 在 Save 提交 patch 并关弹窗 | `enter_on_save_commits_patch` | `crates/tui/src/model_menu/mod.rs` |
| Reasoning 循环仍可用（既有，未回归） | `reasoning_cycle_is_circular` | `crates/tui/src/model_menu/mod.rs` |
| 弹窗锚定定位为纯渲染（无逻辑分支），签名一致由 clippy + 编译保证 | — | `crates/tui/src/command.rs`、`crates/tui/src/model_menu/view.rs` |

- 全量回归：`cargo test --workspace` → 271 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 262 passed | 271 passed |
