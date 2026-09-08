Commit: be76fc1086cbf0d928c1d1e03ad5470563fd86df

# ctl — opencoder-cli 远程管理 CLI

`crates/ctl`（包名/二进制名 `opencoder-cli`）是 `opencoder-server` 控制面的命令行客户端，角色对标 kubectl：覆盖平台全部 HTTP API 并提供 `raw` 逃生舱（任意 method+path 原样直发），保证 100% API 可操作性。不链接 session/store/worker——只依赖 `opencoder-core`（version/fleet 元数据）+ reqwest，纯客户端。

## 结构

- `src/lib.rs` — clap 顶层 `Cli`（全局 flag `--server/--token/--token-file/--verbose` + `--build-info`）与 `Command` 分发到各域模块；`run()` 装配 `Ctx` 并路由。
- `src/ctx.rs` — `Ctx`（server/token/verbose）纯函数解析：flag > 环境变量（`OPENCODER_SERVER_URL`/`OPENCODER_SERVER_TOKEN`）> 报错；`--token` 与 `--token-file` 互斥；URL 去尾斜杠、非 http(s) scheme 拒绝；token 文件读取不回显。
- `src/http.rs` — 纯数据 `RequestPlan`（method/path/query/body，builder 式 `with/with_opt/with_body`）+ `send`（Bearer 头执行）+ `Outcome`（优先取 JSON `error` 字段）+ `exit_code` 分类。
- `src/out.rs` — 输出约定：stdout 每次调用恰一个 JSON 文档（SSE 用紧凑单行 `json_line`）；stderr 是人读 note 与结构化失败 `{"status":..,"error":..}`。
- `src/sse.rs` — 自含最小 SSE 客户端：`FrameAssembler` 逐行拼 `event:/id:/data:` 帧（空行收帧，多行 data 拼接、注释忽略），`print_stream` 每帧打一行 `{"event":..,"seq":..,"data":..}`，Ctrl-C 退出。
- `src/cmd/*` — 按域一个模块，每个模块 = clap `Subcommand` 枚举 + 纯 `plan()`（子命令 → `RequestPlan`，零副作用）+ 薄 `run(ctx, sub)` 执行器；`cmd/mod.rs::exec_plan`（缓冲）与 `exec_stream`（SSE 流式：非 2xx 先 status 预检再按契约分类）是共享执行器，统一套用输出/退出码契约。
- `src/main.rs` — tokio runtime 包裹 `run()`，`Err` 走 `fail_transport` + 退出码 1。

## 命令面

- `health` / `ready` / `time` — 探针（ready 尊重冻结的 drain 模式）与受保护的时钟。
- `drain status|freeze|reopen` — 准入冻结管理。
- `exec …` — executions：list（keyset 分页游标对）/create/get/cmd（cancel/steer 等运行时命令）/events（SSE）/events-page/payload/field/messages/todo-items/project-runs/team-turns（各明细字段的分页读取）/artifact（工件二进制流下载，支持 `-`）。
- `session …` — 舰队级会话：list/create（native 路由）+ get/delete/messages/prompt/events/agent/model/interrupt/fork/compact/handoff/skill/questions/answer/skip/inputs/input-reorder/input-delete/annotation/autopilot/subagents/steer/task（大多经 control relay 转发到所属节点）。
- `nodes …` — list/maintenance/models/skills/dialogs/task-create/task-cancel/task-events。
- `dag defs|dispatch|runs` — 定义 CRUD、派发、run 视图（list/get/events/cancel）。
- `todo envs|tools|templates|run|workflows` — TODO 环境/共享工具/模板版本/派发与工作流运行管理。
- `project overview|goals|milestones|todos` — 项目跟踪全量操作。
- `brain caps|search|plan|plan-get|preview|dispatch` — 能力库 CRUD/绑定、近邻检索、决策树规划与派发。
- `teams list|put` — 团队定义（列表滤掉已退役的 `system` 队）。
- `agents list|create|update|delete|active|meta|resources|nfs` — 内置与自定义 Agent 引用卡及默认 Harness、资源池与 NFS 只读导出生命周期。
- `raw call METHOD PATH [--json …|@file] [--query k=v] [--stream]` — 逃生舱，原样直发任意路由。

## 约定

`--build-info` 在读取 Server 地址和凭据前返回 `opencoder_core::version::build_info_json()`。它与 `opencoder`、`opencoder-server`、`opencoder-agent` 使用相同完整构建元数据；四者由同一发布包安装，清单校验 commit、protocol 与 SPA digest。安装和回滚契约见[平台部署](../../docs/agent-platform.md)。

- 认证：纯 Bearer（与 server/agent 同一 token）；`--json` 值支持 `@file` 读文件，其余按内联 JSON 文本解析，非法即报错不静默。
- stdout 恰一个 JSON 文档（成功 pretty、SSE 紧凑单行）；人读信息与结构化失败只进 stderr，永不混入 stdout。
- 退出码：0 成功 / 1 传输失败（无 HTTP 状态）/ 2 认证失败（401/403）/ 4 服务端拒绝（其余非 2xx）；无子命令 64。流式面（exec events、session events、nodes task-events、dag runs events、todo workflows events、raw --stream）经 `exec_stream` 同样生效。
- `raw` 仅放行 GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS 七个方法，`--query` 必须是 `k=v` 对。

## 测试

`tests/build_info.rs` 验证无 Server 凭据也能读取完整版本信息；`tests/server_local.rs::agents_card_lifecycle_and_active_pointer` 验证内置 Agent 列表、自定义卡片的 Codex 设置和资源引用保留。

- 单测：`src/` 内嵌（http 的 RequestPlan/URL 编码/退出码、ctx 的 flag>env 优先级与互斥、sse 帧解析、out 单行契约、raw 的 body/@file/query/method 白名单、system/brain/project/agents 的 plan 映射，共 24 项）。
- 契约测试：`tests/parse_exec.rs`（16）、`tests/parse_session_nodes.rs`（12）、`tests/parse_dag_teams.rs`（7）、`tests/parse_todo.rs`（7）、`tests/parse_project_brain_agents.rs`（10）——子命令到 `RequestPlan` 的纯映射。
- 集成测试：`tests/server_local.rs`（9）在进程内起真实 control（tempdir + 临时端口 + MockChatClient）打真路由：probes/401→2、drain 周期、teams+raw 等价、dag defs CRUD、todo envs/templates、project goals、brain caps+search、agents 卡片、exec list 空/404→4；`tests/server_local_defs.rs` 是共享基建（合法请求体常量，无测试）；`tests/server_node.rs`（2）是带脚本化 WS 节点（复用 control e2e `MockNode`）的全链路 e2e：exec create relay 往返（节点 journal 为证）、SSE events 终止与 `--after` 续传过滤（进程内 + 真实二进制进程级退出码）、artifact 跨 64KiB chunk 字节保真。
