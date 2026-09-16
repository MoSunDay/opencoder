Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 索引
- `src/bootstrap.rs` — control.db + definitions.db 装配
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- `src/api/executions/` — 派发去重、选点冻结、回执
- `src/api/catalog.rs` — 节点与定义目录
- `src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由
- `src/api/stream.rs` — 分页→SSE 事件流
- `src/api/brain_runs/` — v2 计划注册、能力目录、版本快照、运行提交与持久化后派发。

## Brain 接缝

计划发布检查 Agent/DAG/Team/TODO/Operator 注册身份并固定能力定义、执行配置和资源；相同版本重试复用既有快照。动态生成完整计划后也经过相同发布入口。运行经 `/api/brain/runs` 提交，通用执行 API 拒绝绕过该契约直接创建 Brain。

旧决策树、Playbook 及 Project 的旧 Brain 执行模式返回迁移错误；历史查询保留。节点选点沿用 Fleet 规则，明确指定的子执行位置不会被根节点覆盖。

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
- [agents/brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
