Commit: 1afd5d4375cd10885aee335d3d9dbf9d396bb563

# node 模块

出站 WebSocket：注册、心跳、RPC。

## 索引
- `crates/node/src/fleet/mod.rs` — NodeService trait
- `crates/node/src/fleet/client.rs` — Bearer WS 注册 + 心跳
- `crates/core/src/fleet/protocol.rs` — 协议定义（core）

[NodeOperation::refreshes_inventory](../../crates/core/src/fleet/protocol.rs) 区分只读查询与状态变更；[client.rs](../../crates/node/src/fleet/client.rs) 的只读 RPC 仅回复，不生成全量库存。状态变更、周期心跳与实际库存修订继续报告。

[control](../control/index.md) · [agent](../agent/index.md)
