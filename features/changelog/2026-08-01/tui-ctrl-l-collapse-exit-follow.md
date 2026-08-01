# fix(tui): subagent 退出统一回到底部跟随态（Ctrl+L / Esc），移除 parent 滚动状态恢复

## 背景

Ctrl+L 此前一键承担「折叠所有输出 + 退出子代理视图 + 清空输入」三职，但退出
subagent 时恢复的是进入前的 `parent_scroll` / `parent_follow`，可能停在历史旧位置。
需求是让视图复位键「回到最新内容」：收起所有展开、回到父代理视角、并回到信息展示
最底部（跟随态）。Esc（仅退出）与此一并统一，消除「恢复旧位置」这条死语义。

## 变更

### `crates/tui/src/app_helpers.rs` — `pre_key_intercept`

- **Ctrl+L 分支**：删除 `*scroll = parent_scroll; *follow = parent_follow;` 恢复逻辑，
  改为无条件 `*follow = true;`。配合 `render.rs` 的 follow clamp
  （`if follow { *scroll = max_rows }`），下一次渲染自动把视图钉在最底部。
  无论是否处于 subagent 视角，Ctrl+L 都回到当前视图的最新内容。
- **Esc 分支**：同样改为 `*follow = true;`（回到底部跟随态），不再恢复 parent 状态。
- **死参数清理**：`parent_scroll` / `parent_follow` 不再被任何分支消费，从
  `pre_key_intercept` 签名移除；`scroll` 参数也不再被触碰，一并移除。
- `handle_mouse`：进入 subagent 时不再保存 `parent_scroll/parent_follow`（无消费者），
  两参数从签名移除。

### `crates/tui/src/app.rs`

- 移除 `parent_scroll` / `parent_follow` 局部变量与两处调用参数。

### `crates/tui/src/keybind.rs`

- 帮助文本更新为「退出子代理视图 / 折叠所有输出 / 回到底部跟随 / 清空输入」。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+L 折叠子代理子视图 + 退出 + 折叠父视图 + follow=true + 清空输入 | `ctrl_l_exits_subagent_and_returns_to_follow_mode` | `crates/tui/src/app_helpers_tests/ctrl_l_tests.rs`（unit，新建） |
| Esc 退出 subagent 回到底部跟随态（不折叠、不清输入） | `esc_exits_subagent_and_returns_to_follow_mode` | 同上（新建） |
| 无 subagent 时 Ctrl+L 同样回到底部跟随态 | `ctrl_l_without_subagent_returns_to_follow_mode` | 同上（新建） |
| Ctrl+T 不消费 / Ctrl+L 折叠清空并回跟随态 / Ctrl+F 仅强制重绘不碰 follow | `ctrl_t_not_intercepted_ctrl_l_clears_ctrl_f_redraws`（强化：断言 follow 变迁） | `crates/tui/src/app_helpers_tests/mod.rs`（unit） |
| /config、/model 弹窗内 Ctrl+L 清空聚焦字段（回归，不受影响） | `ctrl_l_clears_active_value` / `ctrl_l_clears_focused_field_and_raw_control_char_forms_match` | `crates/tui/src/model_menu/`（unit） |
| handle_mouse 去参后 滚轮/拖拽/双击 全量回归 | `mouse_*` 系列 | `crates/tui/src/app_helpers_tests/` + `render_tests/arrow_click.rs`（unit） |

## 全量回归

| 检查 | 结果 |
|------|------|
| `cargo check -p opencoder-tui` | PASS |
| `cargo test -p opencoder-tui --lib -- ctrl_l esc_` | PASS — 20 passed / 0 failed |
| `cargo test -p opencoder-tui --lib` 全量 | PASS — 770 passed / 0 failed |
| `cargo test --workspace` 全量 | PASS — 1587 passed / 0 failed / 1 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告 |
