Commit: e50ffc433bca866fd17bd571a74f1bdf17705dea

# node 模块

出站通信层：注册、心跳、索引上报与执行 RPC。

## 关键路径

- `crates/node/src/fleet/mod.rs` — NodeService trait：注册/快照/索引/执行与 brain_frames
- `crates/node/src/fleet/client.rs` — Bearer WS 注册 + 5 秒心跳上报
- `crates/node/src/fleet/cpu.rs` — 进程可用 CPU 与 cgroup quota 算容量
- `crates/core/src/fleet/protocol.rs` — PROTOCOL_VERSION=9、帧与索引定义
- `crates/node/src/runner.rs` — 旧 run_node 队列入口（兼容）
- `crates/node/src/uplink.rs` — 旧 REST Uplink + LocalDagPersistence 接缝
- `crates/node/src/batcher.rs` — 旧 claim/heartbeat 批处理（兼容）

## 边界

- 不持有工作负载；[worker](../worker/index.md) 实现 NodeService。
- 断线仅替换传输，已接受操作脱离连接任务不中止。
- Node 间不直连；维护经 Server 转发 Maintenance RPC。
- Brain 上行帧随报告发送，由 worker 保存并重放至确认；传输层不拥有动作账本。control 异步处理消息，RPC 回执不等待其完成。

## 相关

- [core](../core/index.md) fleet 协议与调度；[control](../control/index.md) Hub
- [dag-runtime](../dag-runtime/index.md) DAG 引擎
