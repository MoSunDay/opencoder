Commit: 9a56d79f7b7e3d67f9e2ed5f3f797571cf5c4793

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码
- `src/cmd/` — 按域子命令（纯 plan() 映射）
- `src/cmd/brain.rs` — brain 子命令，与 Web 共用 API
- `src/cmd/project.rs` — 项目、专项及 TODO API 映射；专项使用 `initiatives` 路由，TODO 查询和编辑使用 `initiative_id`，不提供项目里程碑命令别名；`project todos links/attach/detach/dispatch` 管理原生执行引用和能力派发
- `src/cmd/brain/ontology.rs` — 仅受理 schema_version 7；CLI 读分层视图、层明细及事件，隔离激活通过 OpenCoder session agent loop 输出一次分层模型决策。
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

[connection.rs](../../crates/ctl/src/connection.rs) 为 [ctx.rs](../../crates/ctl/src/ctx.rs) 提供连接默认值：自动读取 `$XDG_CONFIG_HOME/opencoder/ctl.json`，未设置 XDG 时读取 `$HOME/.config/opencoder/ctl.json`；`--client-config` 可指定文件。命令行优先于 `OPENCODER_SERVER_URL` / `OPENCODER_SERVER_TOKEN`，环境变量优先于文件。文件支持 `server`、`token` 或 `token_file` 和 `verbose`，相对 Token 文件路径基于配置文件目录解析；同一来源的 `token` 与 `token_file` 互斥。

## DAG 调用链
- [dag.rs](../../crates/ctl/src/cmd/dag.rs) 将定义、dispatch、运行概况及事件映射到 Server；[workflows.rs](../../crates/control/src/api/compat/workflows.rs) 的 `dag_view` 只投影运行索引、定义和错误，不包含步骤输出。
- [executions.rs](../../crates/ctl/src/cmd/executions.rs) 提供 `exec get` 的执行详情、`exec result` 的节点实时结论与 `exec artifact` 的产物下载；步骤 `output` 由 [dag_steps.rs](../../crates/worker/src/operations/query/dag_steps.rs) 查询。进度、步骤及动态实例端点目前通过 `raw call` 访问。
- [cmd/mod.rs](../../crates/ctl/src/cmd/mod.rs) 按 HTTP 请求是否成功返回退出码；事件流正常结束同样返回 0。CLI 没有等待 DAG 完成并映射任务结论到退出码的专用命令，调用方需分别判断执行状态与步骤输出中的任务结论。

## 相关
- [brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
