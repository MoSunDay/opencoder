# server — `opencoder-server` 二进制

`crates/server`：web 服务器的薄入口（P0 三分叉拆分产物）。组装 `opencoder-web` 的 AppState/router + LibsqlStore + 可选 token 鉴权，监听 HTTP/SSE。**不**链接 dag-runtime/VM/runc 链——DAG 调度在节点执行，server 只做 def/run 管理与事件汇聚（见 [agents/dag](../dag/index.md)、[agents/web](../web/index.md)）。

替代旧 `opencode daemon --server`。DAG 面：`POST /api/dag/defs*`、`POST /api/dag/defs/:id/dispatch`、`GET /api/dag/runs*`、`GET /api/dag/runs/:id/events`（SSE，id=seq，Last-Event-ID 续传）、claim/事件/状态上报端点（`/api/nodes/dag/*`）。
