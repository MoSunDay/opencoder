Commit: 35a3fd069d22848ce0c27d48c6c3f222ccb97f12

# Server 调度总览指标

新增管理员接口 `GET /api/metrics/scheduler` 和 Prometheus 接口 `GET /metrics`。按调度扫描、派发、最近一次日程记录、对应执行结果和在线节点容量分别统计，避免将“派发成功”误当作“执行成功”。扫描与派发计数仅覆盖当前 Server 进程；日程最近状态来自持久化记录。Prometheus 不输出日程或执行 ID 标签。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 集群容量与日程结果 | `reports_cluster_capacity_and_schedule_outcomes_without_task_labels` | `crates/control/tests/e2e/scheduler_metrics.rs` |

- 定向回归：`cargo test -p opencoder-control --lib`、`cargo test -p opencoder-control --test e2e scheduler_metrics`、`cargo test -p opencoder-control --test e2e schedule_api` 通过。
- 全量回归与 clippy：见 [2026-09-29 收口记录](../2026-09-29/release-gate-closure.md)。

当前行为见[Agent 调度平台](../../agent-platform/index.md)，逻辑入口见[control 模块](../../../agents/control/index.md)。
