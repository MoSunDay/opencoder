Commit: (working-tree, pre-initial-commit)

# /mcp 服务器列表启用显式 [ON]/[OFF] 开关指示

## 背景
`/mcp` 列表中每个服务器的启用/禁用状态此前用一个细微的 `●`/空格 标记表示，视觉上不够直观，用户难以一眼分辨开关态。需要更明确的「开关样式」指示。

## 变更
### MCP 列表切换指示
- **`crates/tui/src/mcp_menu/view.rs`**（`render_list`，行 82-117）：
  - 将每行的 `●`/空格 标记替换为显式的 `[ON]`/`[OFF]` 开关 token。
  - 开关 token 使用独立着色：启用=绿色加粗（`Color::Green`），禁用=暗色（`dim_style()`），确认删除态沿用整行红色加粗。
  - 改用 `Span` 拼接行（开关 token / 名称 / transport），名称与 transport 列沿用既有选中/启用样式。
  - 重新对齐列表头为 `on / server / transport` 三列，与数据行列起点一致（开关列宽 5、名称列宽 13）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 切换逻辑（启用→禁用） | `enter_toggles_disabled_when_already_enabled` | `crates/tui/src/mcp_menu/list.rs` |
| 切换逻辑（禁用→启用） | `enter_toggles_enabled_on_selected` | `crates/tui/src/mcp_menu/list.rs` |
| 表单空格切换 enabled | `space_toggles_enabled_field` | `crates/tui/src/mcp_menu/form.rs` |
| JSON patch 写 enabled | `toggle_sets_enabled_flag` | `crates/tui/src/mcp_menu/patch.rs` |

- 目标 crate 回归：`cargo test -p opencoder-tui --lib mcp` → 22 passed; 0 failed（含全部 12 个 mcp_menu 单测）。
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告。
- 构建：`cargo build -p opencoder-tui` → 干净。
- 全量 `cargo test --workspace`：因仓库中另一并行进行中的工作（`crates/session` MCP 客户端特性）持续占用 build lock，未能在本次干净跑通；本变更为 `mcp_menu/view.rs` 单一渲染函数的纯展示层改动，已隔离验证。
- 行数：`crates/tui/src/mcp_menu/view.rs` 229 ≤ 800。

## Impact Surface
- 用户：`/mcp` 弹窗的服务器列表每行开关从 `●` 变为带颜色的 `[ON]`/`[OFF]`，启用态一目了然；交互（Enter 切换、e 编辑、n 新建、d 删除）不变。
- 不影响：CLI、Web、session、store、LLM；开关切换与持久化的 JSON patch 逻辑未改动。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
