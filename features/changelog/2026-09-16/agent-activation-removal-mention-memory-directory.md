Commit: 3fc650d876a10fdcbd24637f46b3b0f47dc4d88a

# Agent 全局激活下线、@//agent 模糊提及与 memory 目录化

全局「激活 Agent」概念删除：新会话默认 agent 改为 `--agent` > `config.agent.default` > `"act"` 三级链，会话级切换（`/act`、`/plan`、`POST /api/sessions/:id/agent`、TUI Shift+Tab）全部保留。聊天输入框支持 agent 模糊提及（SPA `@`/`/agent`、TUI `/agent`）。memory 资源放开为目录化多文件，prompt 注入按字典序聚合。资源文件树与 TUI notepad 的新建/重命名改为行内交互（VSCode 风格）。

- 全局激活下线：删除 `<agents_root>/active` marker 的读取/写入与名字保留分支；`GET /api/agents` 不再返回 `active`，`PATCH /api/agents/active` 移除（404/405 取证）；资源版本写（上传/回滚/卡片 PUT/删除）无条件 `fan_out_reload`；skills 生产发现根回归全局目录；旧安装残留 marker 文件仍被 `list_agents` 跳过。**依赖激活设定默认 agent 的用户需改用 `config.agent.default` 或 `--agent`**（破坏点：`GET /api/agents` 无 `active` 字段、激活端点删除，SPA 同仓库同步）。
- agent 模糊提及：`GET /api/agents` 增加 `description`（prompts 池 `soul.md` 首个非空行，兜底 `Custom agent <name>`）；SPA 新增 `fuzzy.js`（子序列 + 连续/前缀加分，与 TUI `fuzzy_score` 同语义）与 `@` 触发的 agent 菜单，pick 后有会话走 `POST /api/sessions/:id/agent`、无会话 stage 进创建参数；TUI `/agent` 静态命令 + 菜单按名称/描述模糊过滤（名称命中优先于描述命中，tie 保持注册顺序），`@` 文件菜单行为不变；控制头 `/agent <name>` 经 runner `split_control_prefix` 原生识别，协议零改动。
- memory 目录化：写侧放开为任意安全相对路径多文件/子目录（所有 `.md` 强制 UTF-8 + 无 NUL，其余扩展允许二进制；1.5MiB 整包上限语义不变）；prompt 注入递归收集版本目录全部非隐藏 `*.md`，按相对路径字典序以 `\n\n` 拼接，200KiB 字符边界安全截断并追加标记（对齐 `session/src/prompt.rs` `AGENTS_MD_MAX_BYTES`），单 `memory.md` 场景输出与旧实现逐字节一致；`references::scan_memory` 改为目录（递归）含 ≥1 个 `.md` 即命中；SPA memory 页签走通用 FileWorkspace 多文件树。
- 行内文件交互：SPA 资源树（todo/agent 资源共用）新建/重命名行内完成——Enter 提交、Esc/失焦取消，`/` `\` `..` 非法与同名冲突红框拒绝，支持隐式多级路径（`a/b/c.md` 自动补目录），保存仍是整包 PUT（后端零新增端点）；TUI notepad `r` 重命名（目标已存在拒绝且保留输入）、`N` 新建目录、`n` 新建文件保留；agents 列表页新增按名称受控搜索框。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 默认链三级优先级（无激活层） | `effective_default_agent_priority_tiers` | `crates/core/src/agent/tests/file_agents.rs` |
| 旧 active marker 被忽略 | `legacy_active_marker_is_ignored_config_default_decides` | `crates/tui/tests/bootstrap_agent_override.rs` |
| list 不含 active 字段 | `empty_root_lists_cards_only_without_active_field` | `crates/web/tests/web_agents.rs` |
| description=soul 首行+兜底 | `list_items_carry_soul_first_line_description_with_generic_fallback` | `crates/web/tests/web_agents.rs` |
| 激活端点已移除 | `patch_active_endpoint_is_gone` | `crates/web/tests/web_agents.rs` |
| 卡片写无条件 reload | `put_fans_reload_on_every_write`、`delete_card_fans_reload_without_marker` | `crates/web/tests/web_agents.rs` |
| TUI /agent 切换/未知名报错/菜单 pick | `agent_control_head_switches_session_agent`、`unknown_agent_name_errors_and_keeps_current_agent`、`picker_pick_fills_control_head_and_switches` | `crates/tui/tests/agent_mention_flow.rs` |
| SPA @ 菜单模糊列表与 pick 分支 | `fuzzy-lists builtin primary roles first, then registered primary cards`、`fuzzy-matches registered agent names and lists nothing on a miss`、`switches an opened idle session via POST /agent and clears the token`、`stages the pick onto session creation when no session exists` | `crates/web/spa/src/chat/agentPick.dom.test.jsx` |
| 模糊打分语义 | `fuzzy.test.js` 全量用例 | `crates/web/spa/src/fuzzy.test.js` |
| 命令菜单 @ 触发与过滤 | `commandMenu.test.js` 全量用例 | `crates/web/spa/src/commandMenu.test.js` |
| memory 多文件聚合排序 | `memory_multi_file_aggregation_orders_subtree_files` | `crates/core/src/agent/tests/file_agents.rs` |
| 单文件逐字节兼容 | `memory_single_file_matches_legacy_output_byte_for_byte` | `crates/core/src/agent/tests/file_agents.rs` |
| 超 200KiB 截断标记 | `memory_aggregate_over_200kib_is_truncated_with_marker` | `crates/core/src/agent/tests/file_agents.rs` |
| memory 多文件上传/读回/拒绝集 | `memory_pool_accepts_multi_file_trees_and_reads_them_back` 等 | `crates/web/tests/web_agent_resources.rs` |
| control e2e memory 平行用例 | `resource_skills_memory_tools_pools_roundtrip`（memory 段多文件+scan 命中） | `crates/control/tests/e2e/agents_resources_extra.rs` |
| 行内新建嵌套文件/目录/重名拒绝 | `creates a nested file inline from the context menu without a dialog`、`creates a directory inline from a right-clicked directory with a directory placeholder`、`rejects a duplicate sibling name inline with an error style and keeps the draft` | `crates/web/spa/src/ui/files/tree.dom.test.jsx` |
| TUI 重命名/建目录/取消 | `rename_file_flow`、`rename_rejects_existing_target_and_keeps_input`、`create_file_cancelled_by_esc` | `crates/tui/tests/notepad_file_flow.rs` |
| agents 搜索框过滤 | `filters agents by name through the controlled search box` | `crates/web/spa/src/agentsConfig.dom.test.jsx` |

- SPA 全量：`npm --prefix crates/web/spa test` → 798/799 passed（唯一失败 `dag/editor` 连线用例为环境偶发，单独复跑 11/11 通过，与本迭代无关；并发会话同日亦有 770/770 与 31/31 分项取证）。
- SPA 构建：`npm --prefix crates/web/spa run build` 通过，产物与已提交 dist 逐字节一致（sha256 比对），无需重建提交。
- 全量回归：`cargo test --workspace --no-fail-fast -- --test-threads=4` → 406 个测试目标全部 `test result: ok`，5223 passed / 0 failed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 构建：`cargo build --workspace` 通过。
