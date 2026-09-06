Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# control 模块

`opencoder-control` 是平台控制面，独立于本地 session/runtime。由 [server](../server/index.md) 启动，提供 Web、全局定义、节点连接、大脑和调度。

## 边界与数据

- `bootstrap` 只打开新 `control.db` 与 `definitions.db`。前者持久化节点、四字段执行索引和团队/DAG/能力绑定；后者保存大脑及项目定义。平台不连接旧 MySQL 项目库。
- `core::fleet` 定义协议与纯调度函数。Server 不依赖 session、worker、team、project runtime 或 Python VM。
- `transport::Hub` 的连接、负载、待确认 RPC 与容量预留仅在内存；执行明细从拥有 ID 的 Node 读取，不缓存入库。

## 主流程

1. Node 经签名 WebSocket 注册、上报快照及索引；代际和序号约束连接更新，索引归属不可改写。
2. 创建时在 placement 锁内解析全局定义、按 loops/CPU 选择节点、持久化归属并预留；释放锁后发 RPC。
3. Node 接受后释放预留；同 ID 请求转发原节点比较原始输入；超时保留归属，不重新分配。
4. 明细、控制、SSE 和分页产物均通过 ID 路由。旧 Chat/DAG/TODO/Project API 由 `api/compat` 等适配。

普通团队与工作流只分配一个 Node。跨节点 PeerCall 仅允许运行中 system 执行的所属协调节点联系维护 agent。大脑预览不派发，dispatch 绑定目标并沿用稳定请求 ID。项目 `project-<todo-id>` 保持 Plan → Act 节点归属。项目专用路由和通用执行控制共用 `executions::dispatch_command`：每次显式 Plan 解析当前 Server 草稿，覆盖客户端自带快照，再交由原节点执行接收检查。

共享源文件仅复用 Web 的鉴权、静态资源和全局资源管理处理器；平台入口不运行旧 Web 节点任务队列。执行实现见 [worker](../worker/index.md)，业务规则见 [Agent 平台](../../features/agent-platform/index.md)。
