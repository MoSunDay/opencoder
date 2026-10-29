Commit: 基线 f606041c，随本笔提交

# 控制面空闲降本与 CLI 缓冲

继 2026-10-27 空闲 CPU 削减后的第二轮资源开销优化，行为契约不变：

- **Outbox 空闲梯级退避**：`release/outbox.rs` 纯函数 `idle_delay` 连续空闲轮询 1s→2s→5s 封顶，观察到任何工作（分配、到期、重试、资源任务）立即回 100ms；最坏恢复延迟 +5s，远低于 30s 派发时延预算。`hub.views()` 克隆+排序仅在存在待处理分配/资源任务时执行。
- **目录定时器空闲拉长**：`ontology_catalog/sync.rs` 纯函数 `timer_delay`——备忘命中（条目未变且未过 TTL）后睡眠 30s→120s 封顶，任何非命中结果（变化、TTL 到期全量周期、错误）回到 30s；`catalog_changed` 通知唤醒保持即时。快照构建是空闲主导成本，此改动把两次全量源快照的间隔从 30s 拉到 120s，10 分钟治愈周期语义不变。
- **节点注销清理**：`transport/hub.rs` `unregister` 同时清扫该节点的 `accepted` 标记与 `reservations`，消除节点移除后的永久残留；pending 超时路径未动。
- **CLI 双份缓冲**：`ctl/http.rs` JSON 解析成功后清空原始 `body` 字节（`text()` 从已解析值渲染），一次性进程峰值内存减半；单文档 stdout 契约不变。
- **sse.rs 测试修正**：修正 3885164f 引入的两个错误预期（尾部完整行消费后残留缓冲为空；非 JSON 数据经 serde 转义后才是合法 JSON），并补解析断言。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| outbox 空闲梯级 1s→2s→5s 封顶、工作即复位 | `release::outbox::tests::idle_ladder_grows_one_two_five_and_caps_at_five` |
| 目录定时器仅备忘命中后拉长 | `ontology_catalog::sync::tests::timer_delay_extends_only_after_a_memo_hit` |
| 注销清扫本节点 accepted 与 reservations | `transport::hub::tests::unregister_sweeps_per_node_acceptances_and_reservations` |
| JSON body 释放原始字节 / 非 JSON 保留 | `http::tests::{parsed_json_bodies_drop_their_raw_copy, non_json_bodies_keep_their_raw_bytes}` |
| SSE 扫描器残留语义 / 非_JSON 转义 | `sse::tests::{line_scanner_splits_frames_without_per_line_allocations, frame_line_wraps_non_json_data_and_missing_fields}` |

## 验证范围

仅运行本次改动及直接影响链路的定向检查，未运行全量测试。

| 实际命令 / 目标 | 结果 |
| --- | --- |
| `cargo test -p opencoder-control --lib -- release::outbox ontology_catalog::sync transport::hub` | 14 通过（含 3 项新增） |
| `cargo test -p opencoder-cli --lib` | 41 通过（含 2 项新增 + 2 项修正后全绿） |
| `cargo clippy -p opencoder-control -p opencoder-cli --all-targets` | 通过，零警告 |
| 实机复测（负载空闲后）：server(18081) + `--resources`(18082) + agent run，`/api/health` ok，空闲 60s 采样 | server 0.18%→**0.067%**、resources **0.00%**、agent **0.05%**；agent 配置合并 WARN 77 秒内 15 次→3 次（缓存生效，残留为缓存外加载点，非周期性）；服务端日志零 error/panic |
