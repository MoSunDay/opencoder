Commit: c60e2162be48102badf53d8b97e7cfa030b59605

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码
- `src/cmd/` — 按域子命令（纯 plan() 映射）
- `src/cmd/brain.rs` — brain 子命令，与 Web 共用 API
- `src/cmd/brain/ontology.rs` — 仅受理 schema_version 4；CLI 读分层视图、层明细及事件，隔离激活运行分层模型决策。
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

连接参数由 [ctx.rs](../../crates/ctl/src/ctx.rs) 解析：`--server` 优先于 `OPENCODER_SERVER_URL`；`--token` 或 `--token-file` 优先于 `OPENCODER_SERVER_TOKEN`，两个 token 参数互斥。省略 token 参数即可直接使用环境变量。

## DAG 调用链
- [dag.rs](../../crates/ctl/src/cmd/dag.rs) 将定义、dispatch、运行概况及事件映射到 Server；[workflows.rs](../../crates/control/src/api/compat/workflows.rs) 的 `dag_view` 只投影运行索引、定义和错误，不包含步骤输出。
- [executions.rs](../../crates/ctl/src/cmd/executions.rs) 提供 `exec get` 的执行详情与 `exec artifact` 的产物下载；步骤 `output` 由 [dag_steps.rs](../../crates/worker/src/operations/query/dag_steps.rs) 查询。进度、步骤及动态实例端点目前通过 `raw call` 访问。
- [cmd/mod.rs](../../crates/ctl/src/cmd/mod.rs) 按 HTTP 请求是否成功返回退出码；事件流正常结束同样返回 0。CLI 没有等待 DAG 完成并映射业务结论到退出码的专用命令，调用方需分别判断执行状态与步骤输出中的业务结论。

## 相关
- [brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
