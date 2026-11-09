Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# node 模块

Worker 侧节点运行时：向中心 Server 注册，本地执行派发任务并回传事件。

## 索引

- `src/lib.rs` — 节点装配入口
- `src/runner.rs` — 本地会话执行与事件回传
- `src/executor.rs`、`src/batcher.rs` — 任务执行与事件批处理
- `src/control.rs`、`src/uplink.rs` — 控制通道与上行
- [fleet/mod.rs](../../../crates/node/src/fleet/mod.rs) — `NodeService` trait
- [fleet/client.rs](../../../crates/node/src/fleet/client.rs) — Bearer WS 注册、心跳及
  连接内 RPC/报告调度
- [fleet/client/report.rs](../../../crates/node/src/fleet/client/report.rs) — 报告游标与
  逐批发送
- [fleet/client/skip.rs](../../../crates/node/src/fleet/client/skip.rs) — 快照对比纯函数
- [fleet/client/delta.rs](../../../crates/node/src/fleet/client/delta.rs) — 连接内增量基准
- `src/fleet/client/tests/` — 客户端测试（heartbeat / incremental / skip 等）
- [protocol.rs](../../../crates/core/src/fleet/protocol.rs) — 协议定义（core）；
  `NodeOperation` 区分只读查询与状态变更

[control](../control/index.md) · [agent](../agent/index.md)
