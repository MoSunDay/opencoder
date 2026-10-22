Commit: 842e58ef5525e8f189895ad0cc7ac59031f312fc

# node 模块

出站 WebSocket：注册、心跳、RPC。

## 索引
- `crates/node/src/fleet/mod.rs` — NodeService trait
- `crates/node/src/fleet/client.rs` — Bearer WS 注册、心跳及连接内 RPC/报告调度
- `crates/node/src/fleet/client/report.rs` — 保留一个完整报告游标，按原顺序逐批发送
- `crates/core/src/fleet/protocol.rs` — 协议定义（core）

[NodeOperation::refreshes_inventory](../../crates/core/src/fleet/protocol.rs) 区分只读查询与状态变更；[client.rs](../../crates/node/src/fleet/client.rs) 的只读 RPC 仅回复，不生成全量库存。状态变更、周期心跳与实际库存修订继续报告。

清单读取失败时，控制连接继续处理心跳和 RPC，发送 `ready=false` 与实际错误，并保留已知负载计数及 Server 中的执行索引。失败不能发布空的完整清单，也不推进 report ID；下一次心跳或真实修订重新读取，完整成功后才恢复就绪和索引同步。连接关闭仍取消其清单读取与准入处理，传输失败和任务 panic 仍按原错误退出。

[control](../control/index.md) · [agent](../agent/index.md)

旧协议完整报告的负载快照与 Begin 连续发送，随后每次只发送一个索引批次或 Brain 帧，并主动让出执行机会；心跳、读取 RPC 和回复可在批次之间执行。一个连接只保留一份读取中或发送中的报告，后续修订合并为一次待重读请求，不叠加无界发送队列。清单读取与发送耗时、Host 的报告锁、Runtime 清单汇总及 Brain 收集耗时仅记录标识、条数和耗时。

新 Server 通过 WebSocket 升级响应头 `x-opencoder-index-report-ack: 1` 协商报告确认流。Node 仅在协商成功时设置报告的 `acknowledge`，对索引分片逐批等待原报告 ID 与顺序的确认；完整报告提交确认前不收集下一份库存，期间心跳、RPC 和回复继续处理。确认复用连接内 Maintenance 帧，固定 request ID 与报告顺序核验，陈旧确认不推进当前报告。没有协商头时保留原 JSON 与发送方式，协议版本保持 10。

[client/delta.rs](../../crates/node/src/fleet/client/delta.rs) 保存连接内已提交的源游标与 Server 报告序号。Host 提供 `index_changes` 时直接转发变化，不重建完整索引表；其他实现继续使用五字段完整摘要比较。首次连接、重连与每 10 分钟发送全量，其余仅发送新增、修改和移除；无变化只发送负载快照，不发索引报告。最终确认后才推进基准，发送期间的修订合并后重读；确认等待超过 120 秒重连。

双方同时协商 `x-opencoder-index-report-ack: 1` 和 `x-opencoder-index-report-delta: 2` 才启用增量。此模式优先由 `index_changes` 读取变化与负载，缺少此能力才用 `index_report`，Brain 单独收集、发送和重放，不占报告分片确认序号；旧对端沿用全量与原 Brain 顺序。

兼容边界：仅受管 Host 请求增量协商。旧 Server 对直连 Worker 不保存报告提交序号，无法防止迟到旧报告覆盖状态，因此直连 Worker 继续周期性全量；候选提取优化仍生效。Server 拒绝非 Host 代次启用增量。

增量协议 2 依赖 Server 保留每条连接的五字段基准，并在共享提交序号变化时重建完整快照。与只支持协议 1 的 Server 连接时，新 Host 不协商增量，使用带逐批确认的全量报告。全量中的 Brain 帧仍按原顺序发送，但不消耗索引确认序号，也不等待索引确认。发布必须先使新 Host 在当前 Server 完成全量报告，再登记候选 Server，避免旧 Host 的增量基准被候选预热覆盖。
