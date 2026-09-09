Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# agent 模块

opencode-agent 二进制：构造 worker 并接入节点出站通道。

## 关键路径
- `src/main.rs` — clap 参数构造 worker，接入 node WebSocket 通道
- `src/main.rs` `AgentCommand` — `Run`（默认）、隐藏 `InternalProcessSupervisor`（runc 后代）、`dag prepare-rootfs`、`storage migrate-layout`
- `src/storage.rs` — 节点本地存储布局迁移
- token 来自参数或 `OPENCODER_SERVER_TOKEN`，不自动生成
- 关闭：`shutdown_signal` → `drain_shutdown` 有界收尾；超时任务重启标 interrupted，不自动重跑

## 边界
- wasmtime/runc 依赖链只在 agent；server 与主二进制不链接。
- `dag prepare-rootfs` 离线执行，不需要 Server 或模型。

## 相关
- [agents/worker](../worker/index.md) — 实际执行与恢复
- [agents/node](../node/index.md) — 出站 WebSocket 通道
- [agents/dag-runtime](../dag-runtime/index.md) — DAG 调度与执行
