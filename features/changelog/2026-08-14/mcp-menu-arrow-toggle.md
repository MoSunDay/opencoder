# /mcp 弹窗键位调整：←/→ 切换开关（保持打开），Enter/Esc 关闭

## 背景
原 `/mcp` List 视图用 Enter 切换选中 server 的 ON/OFF——toggle 与关闭耦合在同一键上，想连开/关多个 server 必须反复重开弹窗。本轮拆分语义：**方向键负责状态、Enter/Esc 负责退出**。

## 变更
- **`crates/tui/src/mcp_menu/list.rs`**：`handle_key` 主 match 删除 `Enter` toggle 分支，改为 `KeyCode::Enter => (Cancel, None)`（只关闭）；新增 `KeyCode::Left | KeyCode::Right` 分支——本地翻转 `entries[selected].enabled` 后返回 `(Save(toggle_mcp_json), Some(List))`，`state.rs::handle_mcp_key` 把 `Some(next)` 写回 slot，`app_loop_mcp.rs` 的 Save 处理不触碰菜单 → **保存生效且弹窗保持打开**。删除确认子态（Enter='y'）与空列表行为不变。
- **`crates/tui/src/mcp_menu/view.rs`**：标题提示改为 `↑/↓ select, ←/→ toggle, e=edit, n=new, d=delete, Enter/Esc close`。
- Form 变体不动（Enter=保存并关闭、Esc=取消）。

## 附：glm 多模态 + MCP 真机验证（无产品代码改动）
zhipu 标准端点无 `glm-5v`（错误 1214 modelCode 不存在；`/models` 列表亦无 vision id），直接探测确认 **`glm-4.5v`** 可用后替换验证。headless `run --image`（蓝顶栏+红色方块测试 PNG）+ `mcp_servers.mock`（mcp_mock_server）：模型正确描述图片（蓝条/红方块/浅背景），并调用 `mcp__mock__echo {"text":"vision-mcp-ok"}` 返回 `echo: vision-mcp-ok`——vision 分块与 MCP 工具链端到端打通。注意 glm-4.5v 该端点 `max_tokens` 上限 16384。临时目录已清理，密钥全程经 `{ZHIPU_API_KEY}` 间接引用，仓库零残留。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Enter 只关闭不保存 | `enter_closes_menu_without_saving` | tui/src/mcp_menu/list.rs |
| ← 翻转 OFF→ON 且弹窗保持 | `left_arrow_toggles_selected_and_stays_open` | tui/src/mcp_menu/list.rs |
| → 翻转 ON→OFF 且弹窗保持 | `right_arrow_toggles_selected_and_stays_open` | tui/src/mcp_menu/list.rs |
| 连按两次翻回原状态 | `double_right_arrow_reverts_toggle` | tui/src/mcp_menu/list.rs |
| 回归：Esc 关闭 | `escape_cancels` | tui/src/mcp_menu/list.rs |
| 回归：d+y 删除 | `delete_then_y_saves_deletion` | tui/src/mcp_menu/list.rs |
| 回归：↑/↓ 导航 | `arrow_keys_navigate_selection` | tui/src/mcp_menu/list.rs |
| 回归：Save 落盘+reload | `handle_mcp_outcome_success_saves_and_reloads` | tui/src/app/app_loop.rs |

全量回归：`cargo test --workspace` 2541 passed / 0 failed。
