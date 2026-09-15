Commit: 1ac64fe8b81a2c7c144c72b717a8031ab18f2589

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

- 不持有工作负载；[worker](../worker/index.md) 和聚合多个 Runtime 的 [Host](../agent/index.md) 实现 NodeService。
- 断线仅替换传输，已接受操作脱离连接任务不中止；正常 Close 退役与协议/错误关闭分别处理。
- 同一 Node ID 可有主备 Host 通道；索引报告携带递增交接编号，Server 的完整索引确认参与原子交接，旧连接不能回写新状态。
- Node 间不直连；维护经 Server 转发 Maintenance RPC。
- Brain 上行帧随报告发送，由 worker 保存并重放至确认；传输层不拥有动作账本。control 异步处理消息，RPC 回执不等待其完成。

## 相关

- [core](../core/index.md) fleet 协议与调度；[control](../control/index.md) Hub
- [dag-runtime](../dag-runtime/index.md) DAG 引擎
