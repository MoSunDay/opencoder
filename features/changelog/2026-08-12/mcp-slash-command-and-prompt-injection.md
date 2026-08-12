Commit: (working-tree, pre-initial-commit)

# `/mcp` Slash 命令 + MCP Server 配置 + 条件性 System Prompt 注入

## 背景

OpenCoder 此前完全没有 MCP（Model Context Protocol）相关代码（`agents/session/index.md` 列为"非目标"）。本次为后续 MCP 客户端实现铺路：添加 MCP server 配置层、`/mcp` 交互菜单、以及仅当 server `enabled == true` 时将其信息注入 system prompt 的条件性机制。

## 变更

### Config 层（core）
- **`crates/core/src/config/mcp.rs`**（新文件，152 行）：`McpServerConfig` 结构体（`enabled`、`command`+`args`+`env` stdio transport、`url` SSE transport），`#[serde(default)]` 字段，`pub(super) fn merge()` 逐字段 JSON 合并（复用 `providers` 的 `entry().or_default()` 模式，env 值经 `resolve_env` 解析）。
- **`crates/core/src/config.rs`**：`Config` 新增 `mcp_servers: HashMap<String, McpServerConfig>` 字段 + `enabled_mcp_servers()` 辅助方法（返回按名排序的 enabled server 列表）。
- **`crates/core/src/config/merge.rs`**：`merge_into` 添加 `mcp_servers` 合并块；`has_editable_key` 添加非空 `mcp_servers` 检查。

### System Prompt 注入（session）
- **`crates/session/src/prompt.rs`**：`build_system()` 签名新增 `mcp_block: Option<&str>` 参数（第 4 参数），遵循 Active skill 的条件追加模式。新增 `pub fn mcp_section()` 辅助函数：接收 enabled server 列表，返回 `Option<String>`（空列表 → `None`，零行为变更）。
- **`crates/session/src/runner/llm_call.rs`**：`run_one_llm_call` 调用点传入 `mcp_section(&session.config.enabled_mcp_servers())`。
- **`crates/session/src/compaction.rs`**：`estimated_tokens` 调用点同步更新（保持 token 估算一致）。

### Slash 命令 + MCP 菜单（tui）
- **`crates/tui/src/command.rs`**：`COMMANDS[]` 新增 `/mcp`；`SlashAction` 新增 `Mcp` 变体；`parse()` 支持 `"mcp" | "mc"`；`dispatch()` 支持 `"/mcp"`。
- **`crates/tui/src/mcp_menu/`**（新模块，6 文件，纯函数式状态机，对标 `model_menu/`）：
  - `state.rs`（50 行）：`McpMenu` 枚举（`List`/`Form`）、`McpOutcome`（`Idle`/`Save`/`Cancel`）、`handle_mcp_key`（`slot.take()` 所有权转移惯用法）。
  - `list.rs`（146 行）：server 列表 + `handle_key`（↑↓ 选择、Enter 切换 enabled、e 编辑、n 新增、d 删除 + y/n 确认）。
  - `form.rs`（275 行）：`McpForm` 光标式文本编辑器（对标 `provider_form`：Up/Down 切换字段、Left/Right 移动光标、Backspace、Ctrl+U 清空）。
  - `patch.rs`（36 行）：`save_mcp_json`/`toggle_mcp_json`/`delete_mcp_json` JSON merge-patch 构建器。
  - `view.rs`（229 行）：`render_mcp_popup` ratatui 渲染（列表 `●` enabled 标记 + 表单字段高亮 + 光标定位）。
  - `mod.rs`（15 行）：模块入口 + re-exports。
- **`crates/tui/src/app_loop_mcp.rs`**（新文件，61 行）：`handle_mcp_outcome` — `Save` 时 `Config::save` + `Config::load` + `UiCmd::ReloadConfig`（比 model handler 简单：不需要重建 LLM client）。
- **`crates/tui/src/app_loop.rs`**：`dispatch_command` 新增 `mcp_menu` 参数 + `SlashAction::Mcp` 分支 + `app_loop_mcp` 模块注册。
- **`crates/tui/src/app.rs`**：新增 `mcp_menu` slot + 模态优先级链 + `dispatch_command`/`render` 调用线程化。
- **`crates/tui/src/render.rs`** + **`crates/tui/src/frame.rs`**：`render`/`render_frame` 新增 `mcp_menu` 参数 + `render_mcp_popup` 调用。
- **`crates/tui/src/app_helpers.rs`**：`build_system` 调用新增第 4 参数 `None`（token 估算器不需 MCP 信息）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| McpServerConfig 默认禁用 | mcp_server_config_defaults_disabled | config/mcp.rs |
| McpServerConfig serde 往返 | mcp_server_config_roundtrip_serde | config/mcp.rs |
| 默认序列化省略可选字段 | mcp_server_config_disabled_omits_optional_fields_in_json | config/mcp.rs |
| 合并更新 enabled/command | merge_updates_enabled_and_command | config/mcp.rs |
| 合并保留未设字段 | merge_preserves_unset_fields | config/mcp.rs |
| 合并空 command 清除字段 | merge_empty_command_clears_field | config/mcp.rs |
| 合并 args 替换非空 | merge_args_replaces_nonempty | config/mcp.rs |
| mcp_section 空列表返回 None | mcp_section_empty_returns_none | prompt.rs |
| mcp_section stdio transport | mcp_section_disabled_not_included | prompt.rs |
| mcp_section SSE transport | mcp_section_sse_transport | prompt.rs |
| mcp_section 无 transport | mcp_section_no_transport | prompt.rs |
| save_mcp_json 含全部字段 | save_includes_all_fields | mcp_menu/patch.rs |
| save_mcp_json 省略空字段 | save_omits_empty_optional_fields | mcp_menu/patch.rs |
| toggle_mcp_json 设置 enabled | toggle_sets_enabled_flag | mcp_menu/patch.rs |
| delete_mcp_json 发出 null | delete_emits_null_for_key_removal | mcp_menu/patch.rs |
| Enter 切换 enabled | enter_toggles_enabled_on_selected | mcp_menu/list.rs |
| Enter 禁用已启用的 server | enter_toggles_disabled_when_already_enabled | mcp_menu/list.rs |
| 空列表 n 键打开表单 | new_on_empty_list_opens_form | mcp_menu/list.rs |
| Esc 取消 | escape_cancels | mcp_menu/list.rs |
| d→y 删除确认流程 | delete_then_y_saves_deletion | mcp_menu/list.rs |
| ↑↓ 导航选择 | arrow_keys_navigate_selection | mcp_menu/list.rs |
| e 键打开编辑表单 | edit_key_opens_form_with_existing | mcp_menu/list.rs |
| 空名称不保存 | empty_name_does_not_save | mcp_menu/form.rs |
| 输入名称后 Enter 保存 | typing_name_then_enter_saves | mcp_menu/form.rs |
| Tab 循环字段 | tab_cycles_fields | mcp_menu/form.rs |
| 空格切换 enabled | space_toggles_enabled_field | mcp_menu/form.rs |
| Esc 取消表单 | escape_cancels_form | mcp_menu/form.rs |
| parse("/mcp") | parse_mcp_full | command.rs |
| parse("/mc") 别名 | parse_mcp_alias | command.rs |
| dispatch("/mcp") | dispatch_mcp | command.rs |

- 全量回归：`cargo test --workspace` → 2379 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- build：`cargo build --workspace` → Finished
- 行数：config/mcp.rs 152 ≤ 400；mcp_menu/*.rs ≤ 275 ≤ 400；app_loop_mcp.rs 61 ≤ 400；app.rs 798 ≤ 800

## Impact Surface

- 用户：可通过 `/mcp` 命令管理 MCP server 列表（启用/禁用/增删改），配置持久化到 `opencoder.json`。仅有 enabled server 时 system prompt 包含 MCP 段。
- 不影响：Store trait、LLM 后端、Web SSE 协议、CLI 命令结构。MCP 客户端连接/工具发现/工具调用为后续独立任务。

## Related Docs

- [agents/session](../../agents/session/index.md)
- [agents/tui](../../agents/tui/index.md)
- [agents/core](../../agents/core/index.md)
