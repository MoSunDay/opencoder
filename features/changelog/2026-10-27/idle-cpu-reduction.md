Commit: 基线 e0abec72；brain-ACK 修复与空闲开销削减分两笔提交

# 常驻进程空闲 CPU 削减

四个二进制的稳态开销优化，行为与协议契约不变，只减少无变化时的重复工作：

- **节点侧报告跳过**：`crates/node/src/fleet/client/skip.rs` 新增 `PublishedInventory::{capture,covers}` 纯函数，快照对比排除 `sequence` 字段。非增量连接上库存与执行索引均无变化时，报告周期只发送 `NodeFrame::Snapshot`（含负载心跳），不再全量序列化上传库存；独立的 Brain 帧照常排队发送。基线是连接内逐次对比，重连后首份报告仍为全量。
- **Host 采集节拍**：`crates/agent/src/host/mod.rs` 心跳周期 100ms→500ms（仍然远低于 30s 调度时延预算）。
- **Worker 调度节拍**：`crates/worker/src/operations/queue/mod.rs` 自适应：有 pending run 时 100ms，空闲 500ms。
- **控制面 outbox 空闲退避**：`crates/control/src/release/outbox.rs` 无分配、无到期、无重试、无资源任务时轮询 1000ms，否则维持 100ms；判定提取为 `retry.rs::idle()` 纯函数。
- **Ontology 目录同步备忘**：`crates/control/src/ontology_catalog/sync.rs` 条目未变化且距上次全同步 <10 分钟时跳过写租约与 Start/Apply 事务；API 与手动同步路径不变。
- **CLI 渲染降本**：`crates/ctl/src/out.rs` 流式 `to_writer*`；`http.rs` Outcome 缓存 `body: Vec<u8>`、`text()` 惰性解析为 `Cow`；`sse.rs` 游标逐行扫描 + `frame_line()` 纯函数直接透传原始 JSON。stdout 契约（每次调用恰好一份机器可读 JSON 文档）不变。

同时修复 HEAD 既有的报告 ACK 不一致：e0abec72 客户端改为只有索引分片消费 ACK 序列并交错 Brain 帧，但服务端 `socket.rs` 仍在非增量连接上对 Brain 帧自增并回 ACK，导致新节点收到超前的 ACK 序列后断连（`fleet::client::tests::publication` 在 HEAD 确定性失败）。Brain 帧在报告中的交错由 e0abec72 引入，旧节点不会在报告中发送 Brain 帧，因此服务端停止 ACK Brain 帧不破坏发布兼容；增量模式本就不 ACK 独立 Brain 帧。升级顺序约束：混版本窗口中新节点（仅索引分片计 ACK）连旧服务端（仍对 Brain 帧回 ACK）会在 Brain 帧交错时断连重连，滚动发布须先升服务端、后升节点。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 无变化报告只发 Snapshot、负载对比排除 sequence、Brain 帧独立排队 | `fleet::client::tests::skip` |
| ACK 序列只计索引分片（服务端不再 ACK Brain 帧） | `fleet::client::tests::publication` |
| Host 500ms 节拍 | `crates/agent/src/host/mod.rs` 内联测试（28 项） |
| Worker 自适应调度节拍 | `crates/worker/src/operations/queue` 相关用例 |
| outbox 空闲判定 | `release::outbox` 3 项 |
| Ontology 备忘跳过 | `ontology_catalog` 3+8 项 |
| SSE 帧透传 / HTTP 惰性 body | `crates/ctl/src/sse.rs`、`http.rs` 内联测试 |

## 验证范围

仅运行本次改动及直接影响链路的定向检查，未运行全量测试。

| 实际命令 / 目标 | 结果 |
| --- | --- |
| `cargo test -p opencoder-node --lib fleet::client` | 20 通过（含修复后的 publication） |
| `cargo test -p opencoder-control --test node_keepalive` | 11 通过 |
| `cargo test -p opencoder-control --lib release::outbox / ontology_catalog / resource_admission` | 3+11+3 通过 |
| `cargo test -p opencoder-agent --lib host` / `cargo test -p opencoder-worker --lib operations::queue` | 28 通过 / 相关用例通过（既有缺镜像的环境失败不计） |
| `cargo test -p opencoder-cli --lib` | 通过 |
| `cargo clippy -p opencoder-node -p opencoder-control -p opencoder-cli --all-targets` | 通过，零警告 |
| `cargo build --release -p opencoder-server -p opencoder-agent -p opencoder-cli` | 通过 |
| 实机运行：server(18081) + `--resources`(18082) + agent run 连接，`/api/health` ok | 空闲 60s 采样：server 0.18% / resources 0.00% / agent 0.05% CPU，服务端日志零 error/panic |
