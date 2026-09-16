Commit: 272b2b13f497ba839a2e137c55813f9ec3fec7f7

# control 定时调度器与 agent 会话执行面

> 保鲜回填：`272b2b13` 迭代只在同提交内同步了逻辑/能力索引（`agents/control`、`agents/store`、`agents/dag-runtime`、`agents/web`、`features/agent-platform`），未落 dated changelog；本条由仓库记忆保鲜轮次按该提交与 HEAD `134deab2` 代码核对后补记，事实范围以提交与代码为准。

## 定时调度（control）

- 声明式 cronjob：控制面 `schedules.json` 领域配置（`id/cron/kind/target/params/enabled/timezone/overlap`），触发 agent/team/todos/dag/brain 既有执行入口；确定性执行 id `<kind>-<schedule_id>-<scheduled_for_ms>`（幂等，同 tick 重放只记一行）。
- cron 原语在 `crates/core/src/schedule/`（`cron.rs`/`time_param.rs`：时区解析、参数渲染、UTC 对齐）。
- 调度循环 `crates/control/src/scheduler.rs`（仅控制面运行）：默认 15s 扫描；outbox 模式防双启；24h 追赶窗内补跑只取最近一次到期 tick，更老 tick 折叠一条 `missed` 代表行；`overlap: skip` 看上一条 fired 行的执行索引防堆叠；error 行 1h 重试窗内原地重试；单次扫描候选 tick 上限 2000。
- 台账：store `schedule_runs`（fired/missed/error，schema v26；`crates/store/src/schedule_types.rs`、`libsql_store/schedule.rs`、`schema.rs` 迁移）。
- 查询面：`GET /api/schedules`（定义 + last_run/next_run）、`GET /api/schedules/:id/runs?limit=`，admin-only（`crates/control/src/api/schedules/`）；CLI `opencoder-cli schedule list|runs`（`crates/ctl/src/cmd/schedule.rs`）。

## agent 会话执行面

- Operator 页（SPA `chat`）「会话模式」Agent 模式直接以 `kind:'agent'` 发起会话：复用既有会话执行器、无 operator 前导，注册 agent 经 `@` 菜单暂存进创建 body（control `api/session.rs`、web `api_*.rs`）。
- worker `workloads/agent_how.rs`：how_append 预算、有界输出提取、结果/标题形态。
- role gate 放开非管理员 operator/agent 提交与指令（`crates/control/src/role_gate.rs`，kind 检查仍在 executions handler 内做）。

## 兼容修复（同提交）

- compat dialogs 路由补 `DELETE`，修复一键清空走旧兼容路由时的 405（`api/compat/mod.rs`、`api/compat/sessions.rs`）；节点中继化重构见 [chat-clear-node-dialogs.md](chat-clear-node-dialogs.md) 及其中后续补记。
- dag agent 步结构化输出提取增强：尾部裸 JSON 兜底（`extract_tail_bare_json`）+ pin 失败带路径（`crates/dag-runtime/src/exec/agent.rs`，机制见 [dag-runtime 索引](../../../agents/dag-runtime/index.md)）。

## 测试

- control e2e：`schedule_api.rs`（调度 API 与台账）、`sessions_agent.rs`（agent 会话）、`node_dialogs_clear.rs`、`users_api.rs` 角色矩阵同步。
- store：`schedule_runs.rs`、`node_dialogs_clear.rs`、schema 迁移用例同步（`schema_v4_migration.rs` 等）。
- 根级 e2e：`tests/operator_e2e/agent_session.rs`、`gating.rs` 扩展；`tests/dag_e2e/structured_output.rs` 锁裸 JSON 兜底。
- ctl：`parse_schedule.rs`；SPA：`archiveUpload.dom.test.jsx`、`resourceModel.test.js` 等同步。

## Related Docs

- [定时调度能力](../../agent-platform/index.md)、[平台协议文档](../../../docs/agent-platform.md)
- [control 模块](../../../agents/control/index.md)、[store 模块](../../../agents/store/index.md)、[worker 模块](../../../agents/worker/index.md)
- 同提交内 SPA 交互收敛已另有分条：[Operator 入口更名](spa-operator-entry-rename.md)、[导航选择持久化](spa-nav-selection-persistence.md)、[对话面板贴底](chat-sheet-dock-bottom.md)、[资源工具行与压缩包导入](spa-resource-toolbar-and-archive-upload.md)、[大脑一层 KV](brain-launch-engineering-kv.md)；fleet 能力 e2e 套件见 [fleet-capabilities-e2e.md](fleet-capabilities-e2e.md)。

## Release

- `272b2b13` 落 main（HEAD `134deab2` 包含；发布状态以发布历史为准）。
