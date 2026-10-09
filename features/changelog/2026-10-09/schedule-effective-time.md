Commit: 1374d7ed231300bd2d00e13790c6f94da3ac9812

# 定时调度限定生效时间并隔离慢请求

## 原因与修复

- 首次扫描没有执行记录时，原逻辑从最近 24 小时起点计算，把创建前的触发时间也记为 `missed`，并可能立即补跑旧任务；重新启用和修改定义也没有限制历史补跑与失败重试。
- 调度使用当前定义的创建、更新时间作为生效边界；补跑与重试决策集中到纯函数 `scheduler/timing.rs`。相同内容的 PUT、相同启停状态的 PATCH 保留原更新时间。
- 保留未修改任务的 24 小时补跑窗口、只补跑最近一次、1 小时原位重试、重叠控制和手动触发。已有历史记录保留，不新增表或配置。
- 历史查询权限测试同时明确：没有触发记录的 ID 返回空列表，不依赖定义仍然存在。
- 每个定义独立提交，同一 ID 跨扫描最多保留一个进行中的触发；慢准入不再阻塞其他日程，确定性执行 ID 和持久化派发记录继续负责重启去重。
- Host 保留 Runtime 的 HTTP 422 明确拒绝，避免误转为 503 后反复派发相同的无效协议；原执行归属不变。
- 资源快照合并私有暂存目录的同步操作：文件仍逐个落盘，目录自底向上同步后才原子发布，避免每创建一个空目录就重复同步父目录。
- 项目执行集成测试使用独立知识目录，避免继承开发者全局挂载路径导致无关磁盘写回拖慢测试；保留原超时与断言。

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
| 慢准入期间其他日程继续触发，同一定义不重复提交 | `slow_admission_does_not_block_other_schedules_or_duplicate_its_tick` | [isolation.rs](../../../crates/control/tests/e2e/schedule_api/isolation.rs) |
| 协议拒绝保留 422、执行归属及锁释放 | `runtime_schema_rejection_remains_definitive_without_changing_ownership` | [forwarding.rs](../../../crates/agent/src/host/tests/forwarding.rs) |
| 快照完整性、不可变重试、复制失败及软链接拒绝 | `parallel_snapshot_freezes_all_cards_current_versions_and_is_retry_stable` 等 | [snapshot/tests.rs](../../../crates/agents/src/snapshot/tests.rs) |
| Team、DAG、Brain 项目执行使用独立知识挂载 | 集成测试套件 | [executor_team_dag_brain.rs](../../../crates/project/tests/executor_team_dag_brain.rs) |

上述修复通过本批次 Rust 全量回归、Clippy 与格式检查；并行修改后续新增的代码不计入该批次回归。发布仍需同一候选的全站 UI 验收和真实磁盘上 15 分钟稳定观察，功能回归不替代发布验收。构建、运行环境与验证记录保存在仓库外。

## 相关

- [控制面索引](../../../agents/control/index.md)
- [Host 索引](../../../agents/agent/index.md)、[资源快照索引](../../../agents/agents/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
- [调度行为与接口](../../../docs/agent-platform.md)
