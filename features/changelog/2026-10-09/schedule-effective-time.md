Commit: 0b8f947de68c0f06bb2ab7a24c35738c5bcb4cc7

# 定时调度不再补跑生效前的时间

## 原因与修复

- 首次扫描没有执行记录时，原逻辑从最近 24 小时起点计算，把创建前的触发时间也记为 `missed`，并可能立即补跑旧任务；重新启用和修改定义也没有限制历史补跑与失败重试。
- 调度使用当前定义的创建、更新时间作为生效边界；补跑与重试决策集中到纯函数 `scheduler/timing.rs`。相同内容的 PUT、相同启停状态的 PATCH 保留原更新时间。
- 保留未修改任务的 24 小时补跑窗口、只补跑最近一次、1 小时原位重试、重叠控制和手动触发。已有历史记录保留，不新增表或配置。
- 历史查询权限测试同时明确：没有触发记录的 ID 返回空列表，不依赖定义仍然存在。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 新建后等待首次有效触发 | `new_definition_waits_for_its_first_future_tick` | [timing.rs](../../../crates/control/src/scheduler/timing.rs) |
| 修改、重新启用后排除旧触发与旧错误 | `edited_or_reenabled_definition_excludes_old_ticks_and_errors` | 同上 |
| 未修改定义仍补跑，已记录触发不重复 | `downtime_catches_up_only_ticks_after_the_definition` | 同上 |
| 原位重试及一小时截止边界 | `unchanged_definition_retries_in_place_until_the_retry_deadline` | 同上 |
| 时钟回退不提前触发 | `future_definitions_and_ledger_rows_never_fire_early` | 同上 |
| 高频、每日 cron 与未来时间 | `per_second_cron_zooms_to_the_newest_ticks`、`daily_cron_walks_back_within_the_catchup_window`、`at_or_after_now_yields_no_ticks` | 同上 |
| HTTP 新建不产生创建前的历史 | `created_definition_fires_then_patch_disables` | [firing.rs](../../../crates/control/tests/e2e/schedule_api/firing.rs) |
| HTTP 启用、修改不触发旧任务，重复写入不改变起点 | `definition_changes_do_not_backfill_or_retry_old_ticks`、`unchanged_updates_preserve_the_scheduling_baseline` | [timing.rs](../../../crates/control/tests/e2e/schedule_api/timing.rs) |

- 直接引用实际 `scheduler/timing.rs` 的独立 Rust 测试入口：8 passed / 0 failed。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过；本次 Rust 文件格式与 diff 空白检查通过。
- 修复前 HTTP 测试复现了创建前的虚假 `missed`、不应发生的历史派发及重复写入重置时间；修复后的 HTTP 回归未完成。
- 常规测试程序链接长期受主机内存与 I/O 压力阻塞；`cargo test --workspace`、`cargo build --workspace` 的限时尝试均超时，不能记为全量验证通过。未发布。

## 相关

- [控制面索引](../../../agents/control/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
- [调度行为与接口](../../../docs/agent-platform.md)
