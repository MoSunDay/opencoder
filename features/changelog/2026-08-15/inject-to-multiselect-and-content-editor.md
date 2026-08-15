# inject_to 多选对话框与 /cli 多行 content 编辑器

## Context

`/mcp`、`/cli` 表单的 `inject_to` 原为 `parent|subagents|all` 三值循环（Space/Enter 逐个切换），无法只注入 `explore` 而不注入 `build`；`/cli` 的 `content` 是单行输入框，而工具描述通常多行且很长，一行根本放不下。

## Change Summary

- **Core 多选模型**（`crates/core/src/config/cli.rs`）：`InjectionTarget` 由枚举改为 `{parent, explore, build}` 三布尔结构体。序列化为 tag 数组（如 `["explore","build"]`）；反序列化兼容旧值 `"parent"`/`"subagents"`(→explore+build)/`"all"`(→全勾) 且接受数组内混用；parent-only 是 serde 缺省，序列化时省略 `inject_to` 字段。`allows(mode)` 升级为 `allows_agent(name, mode)`：Primary 一律看 `parent` 位，Subagent 按名字精确匹配 `explore`/`build`。`label()` 返回 `parent+explore` 样式 String。两处 `merge()`（cli/mcp）同时接受字符串与数组 patch 值。
- **过滤链路**：`Config::enabled_mcp_servers_for` / `enabled_cli_for` 改收 `(name, mode)`；`runner/llm_call.rs`（cli 段、`mcp_tool_allowed`、`mcp_status_for_agent`）与 `compaction.rs` 全部传入 `session.agent.name`，注入粒度从「父/子两级」细化到「explore 与 build 独立勾选」。workflow 调度 Agent 仍被显式排除。
- **共享勾选对话框**（新模块 `crates/tui/src/scope_dialog/`）：`[x]/[ ]` 复选列表叠加渲染在表单之上。↑/↓（含 Tab）移动、Space 勾选、Enter 确认（空选无效——必须至少勾一项，否则等同 disable——对话框保持打开并显示提示行）、Esc 取消；对话框打开时粘贴被吞掉。
- **表单集成**：`McpForm`/`CliForm` 持有 `scope_dialog`；聚焦 InjectTo 字段 Enter/Space 弹出对话框（替换原 cycle 行为）。
- **content 多行编辑器**（新文件 `crates/tui/src/cli_menu/content_dialog.rs`）：聚焦 Content 字段按 Enter 弹出。框内字符输入、Enter=换行（不承担保存）、Backspace、←/→ 单字符移动、↑/↓ 按行列移动（保持列，越界落文本尾）、Ctrl+U 清空、滚动跟随光标（8 行视口）；Ctrl+S 应用回写表单，Esc 取消。`CliMenu::paste` 在对话框打开时原样插入含换行的粘贴内容；折叠字段保留单行预览（`display_content`）。
- 顺带修复 session 既有失败测试 `mcp_tools_visible_to_act_agent`（HEAD 上即失败：注册表塞了假 MCP 工具但 config 未登记 server，导致过滤为空；现为该测试补上 parent 范围的 server 配置）。

## Validation

| 验证点 | 测试 |
| --- | --- |
| 序列化为数组 / parent-only 缺省省略 / roundtrip | `serializes_as_tag_array`、`parent_only_is_default_and_roundtrips`、`cli_config_omits_parent_only_inject_to`、`cli_config_roundtrips_multi_target` |
| 旧值 `"parent"/"subagents"/"all"` 加载语义不变 | `loads_legacy_string_values`、`legacy_subagents_value_loads_and_filters_to_both_subagents`、`legacy_all_value_loads_into_every_agent` |
| 数组解析与未知 tag 忽略；merge 接受字符串/数组 | `loads_tag_arrays_and_ignores_unknown_tags`、`merge_accepts_string_and_array_inject_to`（cli）、`merge_accepts_legacy_string_and_array_inject_to`（mcp） |
| `allows_agent` 名字粒度矩阵；config 过滤函数 | `allows_agent_matrix`、`enabled_for_filters_by_agent_name_within_subagents`、`parent_flag_covers_every_primary_agent` |
| session 粒度注入（CLI 段/MCP 工具仅 explore 或仅 build 可见） | `cli_injected_only_into_explore_subagent_by_name`、`mcp_tools_scoped_to_single_subagent_by_name` |
| 既有 MCP 工具可见性行为不回归（含修复） | `mcp_tools_visible_to_act_agent`、`mcp_tools_hidden_from_subagent`、`mcp_tools_hidden_from_workflow_agent` |
| 勾选对话框键位（勾选/环绕/确认/空选拒绝/Esc） | `scope_dialog::tests` 6 例 |
| 勾选对话框渲染叠加与极小终端 clamp | `renders_checkbox_rows_and_survives_tiny_area` |
| /cli、/mcp 表单弹窗开合与选择回写、粘贴路由 | `cli_menu::form::tests` 8 例、`mcp_menu::form::tests` 2 例 |
| 多行编辑器（输入/换行/退格/行列移动/键位/滚动/多字节/粘贴） | `content_dialog::tests` 8 例 |
| label 展示 | `label_joins_selected_tags` |

全量门禁（当次实跑，串行两次复跑完全一致 2652/2652）：`cargo test --workspace` → 2652 passed / 0 failed（163 个 test result 全 ok）；`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；`cargo build --workspace` → Finished（release 亦通过）。

新增测试 41 例（core 14、session 2、tui 25），另有 1 例既有失败 `mcp_tools_visible_to_act_agent`（HEAD 上即失败，根因是测试注册了假 MCP 工具但 config 未登记对应 server）本次顺带修复；无测试删除、无 `#[ignore]` 新增、无断言弱化。真实模型 e2e（`scripts/e2e_glm.py --skip-web`，glm5.2）：60 passed / 0 failed / 1 skipped。

## Related Docs

- [逻辑：core](../../../agents/core/index.md)、[逻辑：tui](../../../agents/tui/index.md)、[逻辑：session](../../../agents/session/index.md)
- 前序：`cli-registry-and-agent-injection-scope.md`
