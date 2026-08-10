Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# 修复 subagent 等待型 steer 在终止边界丢失

## Context

聚焦运行中 subagent 按 Enter 会把 steer 写入 child session，并等待当前 turn 自然结束；`>` 则立即打断。旧实现的 Enter admission 与 child 最终 `Done` 没有生命周期同步，因此输入可能在最后一次 pending 检查之后写入，界面显示已提交但再无 runner 提升该 row。

## Change Summary

- 每个 live child 增加 admission gate：异步写入先 reservation，成功后提交 epoch。
- child idle 终止与 admission 原子结算：reservation 先发生则继续并消费，关闭先发生则拒绝写入。
- TUI 终态竞态恢复输入并保留图片；Web 返回 409。写入后遭强制关闭时回滚 row。
- `>` 的立即 turn cancel 与 Enter 的自然等待语义保持分离。

## Impact Surface

- Session runner 的 subagent 生命周期与强制清理注册表。
- TUI focused-subagent Enter admission。
- Web subagent steer endpoint 的 409 错误语义。

## Compatibility

不新增数据库字段、迁移或环境变量；成功 HTTP 响应格式不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| admission commit 推进 epoch | `committed_admission_forces_another_boundary` | `subagent_steer_gate.rs` |
| idle 等待在途写入 | `idle_waits_for_an_in_flight_admission` | `subagent_steer_gate.rs` |
| close/force-close 拒绝迟到写入 | `close_rejects_late_admission` / `force_close_makes_existing_reservation_fail_commit` | `subagent_steer_gate.rs` |
| Web stale running row 返回 409 | `steer_running_row_without_live_gate_returns_409_and_does_not_admit` | `web/tests/subagent_steer_api.rs` |
| `/task` 切换同步重绑 child registries | `rebind_session_swaps_the_active_cancel_token` | `tui/src/worker/tests.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- 数据与配置：无 schema、迁移、数据库删除、环境变量或公开成功响应变化；终态拒绝可由现有 409/错误状态观测。

## Related Docs

- [session 模块](../../../agents/session/index.md)
- [tui 模块](../../../agents/tui/index.md)
- [web 模块](../../../agents/web/index.md)
