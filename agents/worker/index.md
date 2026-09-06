Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# worker 模块

`opencoder-worker` 持有节点执行数据和工作负载适配器，实现 [node](../node/index.md) 的 `NodeService`。由 [agent](../agent/index.md) 二进制构造。

## 所有权

`WorkerOptions` 指定独立数据目录、工作目录、顶层容量和 DAG 能力。节点 ID 持久化、目录加锁。`runtime.db` 保存 session/workflow/project 明细，`executions/` 保存原始请求、定义快照与状态日志，资源、团队和产物目录均在 Node。

`operations/create` 在 admission 锁内去重、预检、固定资源、取得容量并 fsync 接受记录；`launch` 执行并持久化结果。Plan 快照仅在忙碌、容量、类型、目标和资源检查全部通过后写入接收记录；拒绝请求不改变 journal。新快照预检失败会清理未接收副本，允许同 ID 重试读取最新资源。运行和控制不依赖 WebSocket 存活。重启未完成记录变 interrupted，显式 resume 才运行；落盘故障让节点不可调度。

## 执行适配

- agent：复用 [web](../web/index.md) 本地 session API、drain 和消息事件持久化；HTTP router 在进程内调用，Node 不开放入站 HTTP。
- 普通 team：自定义 `TeamDispatcher` 为每个成员创建本地会话；职责与定义固定。
- system team：协调记录本地保存，远端成员通过 PeerBridge 调用各节点维护 agent，维护明细仍属远端。
- TODO：[todos](../todos/index.md) Runtime 的父会话与子执行都使用本地 Store。
- DAG：[dag-runtime](../dag-runtime/index.md) 的 uplink 接到本地持久化，产物经命令分页读取；resume 跳过已成功落盘的检查点。
- project：[project](../project/index.md) Runtime 使用节点库，Plan 文本、执行记录和子会话均本地；项目结构来自 Server 快照。

`resources` 对新执行校验显式资源目录为只读 NFS，再复制当前资源文件形成不可变快照；失败拷贝清理 staging。已有执行继续或恢复直接复用本地快照，NFS 不可用时节点拒绝新执行。已接收会话尚未建立 session 时，事件入口返回合法空流而不是 404。`core::agent::scope` 与 session runner 传播任务局部资源根，避免跨并发执行串用版本。`session::loop_registry` 提供真实活跃 loop；顶层容量与 loop 计数分开。

维护工具通过 session extension 注册，只在维护会话中暴露真实状态、配置和任务控制接口；注册与心跳不会自动发起修复。业务规则见 [Agent 平台](../../features/agent-platform/index.md)。
