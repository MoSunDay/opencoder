Commit: 4c174685c758d7dabd7503d708917a560f125c4d

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

`--no-auth` 明确省略 Authorization，连接文件可用 `auth_enabled:false`；默认仍要求既有 Token。免鉴权不读取 Token 文件，显式 Token 参数覆盖文件设置，`--no-auth` 与显式凭据互斥。项目三层增删改查与归属操作见 [用法](../../docs/project-cli.md) 和 [opencoder-projects skill](../../skills/opencoder-projects/SKILL.md)。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码
- `src/cmd/` — 按域子命令（纯 plan() 映射）
- `src/cmd/brain.rs` — brain 子命令，与 Web 共用 API
- `src/cmd/project.rs` — 项目、专项及 TODO API 映射；专项使用 `initiatives` 路由，TODO 查询和编辑使用 `initiative_id`，不提供项目里程碑命令别名；`project todos links/attach/detach/dispatch` 管理原生执行引用和能力派发
- [project.rs](../../crates/ctl/src/cmd/project.rs) 的 `project people` 映射只返回用户名的 `/api/project/people`；三层创建与修改 JSON 透传独立的 `owner_user_name`、`follower_user_names`。
- [project/comments.rs](../../crates/ctl/src/cmd/project/comments.rs) — `project todos comments list/add` 映射分页与追加接口，接受 `--json @file`，请求 ID 由调用方固定以支持重试；概述收集沿分页读完评论，游标不前进明确报错。
- [project/context.rs](../../crates/ctl/src/cmd/project/context.rs) — `project todos context` 读取任务、全部评论与关联执行，默认包括全部历史；`--execution` 仅选择已关联的一次执行。复用原生证据分页读取详情、Session 消息、事件和省略字段，保留原 Runtime 归属；部分读取失败输出 `complete:false` 与 `issues`，退出码 4，不写回任务。`project todos messages --seq/--offset` 对应原生消息游标，旧参数名保留别名。
- [project/summary/](../../crates/ctl/src/cmd/project/summary/) — `project initiatives summary-context` 读取全部 TODO、全部分页评论、去重后的全部执行引用和原生证据；消息、事件、Team 轮次、TODO 项及省略正文按原生游标读取，失败保留来源和原因。`project goals summary-context` 返回全部专项及概述，提示缺失或过时内容；调用方总结后通过已有 PATCH 写入，见 [概述用法](../../docs/project-summaries.md)。`summary-versions` 查看最近 10 个版本，`summary-restore` 调用服务端将历史正文恢复为新版本。
- `src/cmd/brain/ontology.rs` — 仅受理 schema_version 7；CLI 读分层视图、层明细及事件，隔离激活通过 OpenCoder session agent loop 输出一次分层模型决策。
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

[connection.rs](../../crates/ctl/src/connection.rs) 为 [ctx.rs](../../crates/ctl/src/ctx.rs) 提供连接默认值：自动读取 `$XDG_CONFIG_HOME/opencoder/ctl.json`，未设置 XDG 时读取 `$HOME/.config/opencoder/ctl.json`；`--client-config` 可指定文件。命令行优先于 `OPENCODER_SERVER_URL` / `OPENCODER_SERVER_TOKEN`，环境变量优先于文件。文件支持 `server`、`token` 或 `token_file` 和 `verbose`，相对 Token 文件路径基于配置文件目录解析；同一来源的 `token` 与 `token_file` 互斥。

项目上下文的执行类型以原生详情中的 `execution.kind` / `request.kind` 为准；仅在详情读取失败或缺失类型时使用关联中的提示。过期或空类型不能使已存在的 Agent/Operator 消息被跳过。`messages` 直接按已关联的执行 ID 请求原生接口，由 Server 判定是否支持会话读取。

## DAG 调用链
- [dag.rs](../../crates/ctl/src/cmd/dag.rs) 将定义、dispatch、运行概况及事件映射到 Server；[workflows.rs](../../crates/control/src/api/compat/workflows.rs) 的 `dag_view` 只投影运行索引、定义和错误，不包含步骤输出。
- [executions.rs](../../crates/ctl/src/cmd/executions.rs) 提供 `exec get` 的执行详情、`exec result` 的节点实时结论与 `exec artifact` 的产物下载；步骤 `output` 由 [dag_steps.rs](../../crates/worker/src/operations/query/dag_steps.rs) 查询。进度、步骤及动态实例端点目前通过 `raw call` 访问。
- [cmd/mod.rs](../../crates/ctl/src/cmd/mod.rs) 按 HTTP 请求是否成功返回退出码；事件流正常结束同样返回 0。CLI 没有等待 DAG 完成并映射任务结论到退出码的专用命令，调用方需分别判断执行状态与步骤输出中的任务结论。

## 相关
- [brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
