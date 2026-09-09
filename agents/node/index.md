Commit: (working-tree, 基于 303b95027b49873a9833393d57b72de68747a9e0)

# node 模块

平台的出站通信层位于 `crates/node/src/fleet/`。它不持有具体工作负载；[worker](../worker/index.md) 实现 `NodeService` 提供注册、负载、索引及执行 RPC。

## 通道

- `client` 使用 Bearer 认证建立 WebSocket 通道，注册携带 Node ID 与协议版本（v6）。
- 5 秒心跳与 loop 变化通知上报快照和五字段索引；快照包含 active_runs、max_runs、pending_runs 和 queue_order；RPC 后先同步最新负载/索引，再回复确认。
- 网络操作有超时、帧大小和并发上限。通道重连仅替换传输；已接受的操作脱离连接任务，断线不会中止节点执行。
- `PeerBridge` 将 system 协调节点的维护请求转发给 Server；Server 验证协调执行的归属和状态，Node 间不直接连接。
- `cpu` 从进程可用 CPU 与 cgroup quota 计算可用容量，支持小数 CPU。

协议及调度纯函数位于 [core](../core/index.md) `fleet`；Server 连接/预留状态位于 [control](../control/index.md)。

## 兼容接口

旧 `run_node`、REST Uplink、claim/heartbeat/batcher 和 DagHook 留在库中供兼容接口与原测试使用，平台二进制不走该队列。Uplink 的 `LocalDagPersistence` 接缝供新 worker 把 DAG 事件、结果写在节点，避免向 Server 上传明细。DAG 引擎见 [dag-runtime](../dag-runtime/index.md)。
