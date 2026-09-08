Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# opencoder-cli 远程管理 CLI（crates/ctl）+ crates/cli 更名 opencoder-local

## 问题与行为

平台此前没有面向 `opencoder-server` 的命令行客户端：运维/脚本只能手写 curl（拼 Bearer 头、记路由、解析 HTML/JSON 混合输出）。本轮新增 `crates/ctl`——包名/二进制名 **`opencoder-cli`**，kubectl 角色的控制面客户端，覆盖 server 全量 API；同时把原 `crates/cli` 更名为 `crates/local`（包名 `opencoder-local`），`opencoder-cli` 名称让给远程 CLI，本地前端行为不变并新增 `opencoder --cli` 兼容别名（global bool，无行为影响）。

## 命令面（按域）

- `health`/`ready`/`time`/`drain status|freeze|reopen` — 探针与准入冻结管理。
- `exec` — executions 全量：list（keyset 分页）、create、get、cmd（cancel/steer）、events（SSE）、events-page、payload、field、messages、todo-items、project-runs、team-turns、artifact（二进制流下载）。
- `session` — 舰队级会话 relay：list/create/get/delete/messages/prompt/events/agent/model/interrupt/fork/compact/handoff/skill/questions/answer/skip/inputs/input-reorder/input-delete/annotation/autopilot/subagents/steer/task。
- `nodes` — list/maintenance/models/skills/dialogs/task-create/task-cancel/task-events。
- `dag`（defs/dispatch/runs）、`todo`（envs/tools/templates/run/workflows）、`project`（overview/goals/milestones/todos）、`brain`（caps/search/plan/plan-get/preview/dispatch）、`teams`（list/put）。
- `agents` — 引用卡 list/create/update/delete/active/meta + resources 资源池 + nfs 导出生命周期。
- `raw call METHOD PATH [--json …|@file] [--query k=v] [--stream]` — 逃生舱，任意路由原样直发，兜底 100% API 可操作性。

## 关键设计

- **plan 纯函数**：每个域模块是 clap `Subcommand` + 纯 `plan()`（子命令 → `RequestPlan` 纯数据）+ 薄 `run(ctx, sub)`；`cmd/mod.rs::exec_plan`（缓冲）与 `exec_stream`（SSE 流式）两个共享执行器统一套输出/退出码契约。映射可无 IO 单测，执行器只有两个。
- **退出码契约**：0 成功 / 1 传输失败 / 2 认证失败（401/403）/ 4 服务端拒绝（其余非 2xx）；无子命令 64。缓冲与流式面一致：`exec_stream`（session/nodes/dag/todo/raw/exec 全部流式入口共用）对非 2xx 先做 status 预检再分类，不再落到传输错误的退出码 1。
- **stdout 单 JSON**：每次调用 stdout 恰一个可解析 JSON 文档（供代理消费）；人读 note 与结构化失败 `{"status":..,"error":..}` 只进 stderr。
- **SSE 行格式**：`events` 等流式命令每帧打一行紧凑 `{"event":..,"seq":..,"data":..}`（自含最小 SSE 解析器，不链接 llm crate）。
- **raw 兜底**：仅放行 GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS；`--json` 支持内联或 `@file`，非法 JSON 报错不静默；`--query` 必须为 `k=v` 对。
- 认证：纯 Bearer（与 server/agent 同一 token）；配置 flag > 环境变量（`OPENCODER_SERVER_URL`/`OPENCODER_SERVER_TOKEN`）。

## 更名：crates/cli → crates/local

原 `crates/cli`（包名 `opencoder-cli`）更名为 `crates/local`（包名 `opencoder-local`），仍是根二进制 `opencoder` 的本地前端，行为不变。`opencoder --cli` 兼容别名恒可达本地前端（`opencoder --cli run …` 与 `opencoder run …` 解析等价）；`opencoder-cli` 名称此后专指远程管理 CLI。记忆同步：`agents/cli` → `agents/local`（git mv），交叉链接（agents.md、agents/todos、agents/store、features/index、features/todos）已改；新增 [agents/ctl](../../../agents/ctl/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| RequestPlan 构造/URL 查询编码/退出码分类/错误信息提取 | `plan_url_encodes_query` 等 4 项 | `crates/ctl/src/http.rs` |
| Ctx 解析：flag>env 优先级、URL/token 缺失报错、token 与 token-file 互斥、非 http scheme 拒绝 | `flags_win_over_env` 等 5 项 | `crates/ctl/src/ctx.rs` |
| SSE 帧解析：event/id/data、多行 data 拼接、连续空行不产空帧 | `parses_event_id_data_frame` 等 3 项 | `crates/ctl/src/sse.rs` |
| stdout 单 JSON 文档/单行契约 | `json_line_is_single_compact_line` | `crates/ctl/src/out.rs` |
| raw 兜底：body 内联/@file、query k=v 校验、method 白名单组装 | `body_inline_and_at_file` 等 3 项 | `crates/ctl/src/cmd/raw.rs` |
| system 探针与 drain plan 映射 | `probes_are_plain_gets` 等 2 项 | `crates/ctl/src/cmd/system.rs` |
| brain/project/agents plan 映射 | 各 2 项 | `crates/ctl/src/cmd/{brain,project,agents}.rs` |
| exec 各子命令 → RequestPlan 映射（16 项） | `list_maps_filters_and_full_cursor` 等 | `crates/ctl/tests/parse_exec.rs` |
| session/nodes plan 映射（12 项） | 全部 | `crates/ctl/tests/parse_session_nodes.rs` |
| dag/teams plan 映射（7 项） | 全部 | `crates/ctl/tests/parse_dag_teams.rs` |
| todo plan 映射（7 项） | 全部 | `crates/ctl/tests/parse_todo.rs` |
| project/brain/agents plan 映射（10 项） | 全部 | `crates/ctl/tests/parse_project_brain_agents.rs` |
| 集成：probes/Bearer 401→退出码 2 | `system_probes_and_bearer_auth_contract` | `crates/ctl/tests/server_local.rs` |
| 集成：drain freeze/reopen 真实周期 | `drain_cycle_against_the_real_admission_gate` | 同上 |
| 集成：teams put 与 raw 逃生舱等价 | `teams_put_list_and_raw_escape_hatch` | 同上 |
| 集成：dag defs CRUD 往返 | `dag_definitions_crud_roundtrip` | 同上 |
| 集成：todo envs/templates 全生命周期 | `todo_envs_and_templates_full_lifecycle` | 同上 |
| 集成：project goals CRUD | `project_goals_create_list_patch_delete` | 同上 |
| 集成：brain caps 生命周期 + search | `brain_caps_lifecycle_and_search` | 同上 |
| 集成：agents 卡片生命周期 + active 指针 | `agents_card_lifecycle_and_active_pointer` | 同上 |
| 集成：exec list 空结果 + 404→退出码 4 | `exec_list_empty_and_missing_get_rejected` | 同上 |
| 集成：SSE 流式退出码契约（exec events 401→2/404→4、raw --stream 401→2） | `system_probes_and_bearer_auth_contract`（扩展断言） | `crates/ctl/tests/server_local.rs` |
| e2e：带 WS 节点（复用 control e2e MockNode）exec create relay 往返 + SSE events 终止 + `--after` 续传 + artifact 跨 64KiB chunk 字节保真 | `exec_create_reaches_the_node_and_streams_events` | `crates/ctl/tests/server_node.rs` |
| e2e：真实二进制进程级 SSE 帧过滤（`--after 2` → 仅 seq 3）与退出码（成功 0 / 错误 token 2） | `sse_after_resume_and_exit_codes_process_level` | 同上 |
| 集成基建（进程内 control harness + 合法请求体常量，无测试） | — | `crates/ctl/tests/server_local_defs.rs` |

统计：src 内嵌单测 24 项 + 契约 52 项（16+12+7+7+10）+ 集成 11 项（server_local 9 + server_node 2）。

## 验证

- 全量回归：`cargo test --workspace --no-fail-fast` → 346 个测试目标全绿（4904 passed / 0 failed）
- Lint：`cargo clippy --workspace --all-targets` → EXIT=0，零警告

## 相关文档

- [ctl 模块](../../../agents/ctl/index.md)
- [local 模块](../../../agents/local/index.md)（原 agents/cli）
- [Web 模块](../../../agents/web/index.md) / [Server 模块](../../../agents/server/index.md)
