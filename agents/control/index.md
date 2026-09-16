Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# control 模块

平台控制面：节点接入、全局定义、执行索引与调度。

## 索引
- `src/bootstrap.rs` — control.db + definitions.db 装配
- `src/transport/hub.rs` — Node WS Hub（协议校验、RPC）
- `src/api/executions/` — 派发去重、选点冻结、回执
- `src/api/catalog.rs` — 节点与定义目录
- `src/api/compat/` — 旧 Chat/DAG/TODO/Project 兼容路由
- `src/api/stream.rs` — 分页→SSE 事件流

## 相关
- [agents/node](../node/index.md)、[agents/worker](../worker/index.md)
