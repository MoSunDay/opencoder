Commit: (working-tree, post-860831d)

# steer/queue 提交"落库但永不消费"的 drain 滞留（TUI idle 重启 + Web watcher 尊重 interrupt）

## Context

两条提交链路都存在"input 已持久化、但没有任何 drain 会再消费它"的窗口：

1. **TUI**：Tab-queue / Enter-while-running 经离环 admitter actor 落库。若落库完成时 UI 已 idle（runner 的 turn 恰好越过最后一次 pending 检查，典型于 cancel/interrupt 收尾竞速），`AdmitDone` 分支只对账队列镜像，不重启 drain——行永久滞留，直到用户下次手动提交。Done 边界的 `drain_pending` 兜底（本轮早些时候已加）覆盖"store 已有行、Done 才发现"的方向，但覆盖不了"Done 已过、行后落库"的方向。
2. **Web**：运行中 admit 会 spawn watcher，轮询 drain 退出后复查 pending 兜底重启。但 drain 若是被 `POST /interrupt`（或 `DELETE /session`）硬取消打断的，queue 行仍然 pending，watcher 会**换新 cancel token 复活已被用户显式停止的 run**——interrupt 语义被兜底逻辑击穿。

## Change Summary

- `crates/tui/src/idle_rekick.rs`（新模块，330 行内）：`stranded_pending(store, sid)`（store 权威复查 Queue+Steer 两路 pending；读错误按无滞留处理，fail-closed 由下一 Done/TurnDone 边界自愈）+ `on_admit_done(...)`（对账 + A1 滞留重启：成功 admit 且 UI idle 且 store 仍有 pending → 空 prompt 重启 drain，返回 `AdmitDoneFlow::{Ok, Started, WorkerDead}` 供主循环翻转 running/follow/begin_turn）。
- `crates/tui/src/app.rs`（admit_done_rx 分支）：改为单点调用 `idle_rekick::on_admit_done`，app.rs 保持 800 行帽（anim_ticker 分支压缩 3 行腾位）。
- `crates/web/src/handle.rs`（watcher 重启路径）：赢得 `draining` swap 后先读当前 cancel token，`is_cancelled()` 则复位 draining 并放弃重启——用户的停止优先，pending 行留给下次用户主动 admit 的 drain。interrupt 与 DELETE 两种硬取消一并覆盖。
- 稳定层同步：`agents/tui/index.md`（queue_admitter 条目补 A1 滞留重启）、`agents/web/index.md`（admit_and_drain 条目补 watcher 的 interrupt-不复活规则）。

## Validation（当次实跑）

- `cargo test --workspace`：**2985 passed / 0 failed**。
- `cargo clippy --workspace --all-targets`：0 warning / 0 error。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `tui idle_rekick::tests::idle_admit_with_pending_row_restarts_drain` | unit | idle + store 有 pending 行 → `Started`，镜像 temp→真实 seq 原位改写，命令流为 ResetCancel + 空 Prompt（drain 重启） |
| `tui idle_rekick::tests::running_or_failed_admit_does_not_rekick` | unit | drain 在跑 → `Ok` 零命令；失败 admit（回滚 + flash）idle 也不重启 |
| `tui idle_rekick::tests::idle_admit_with_nothing_pending_is_plain_ok` | unit | idle 但 store 空（行已被消费）→ `Ok`，无多余 drain turn |
| `tui idle_rekick::tests::pending_queue_row_is_stranded` / `empty_store_reports_no_stranded_rows` / `missing_session_reports_no_stranded_rows` | unit | store 权威复查三态：有行/空/会话缺失 |
| `web tests/interrupt_beats_pending_replay.rs::interrupt_cancels_drain_watcher_does_not_resurrect` | integration | 挂起 LLM turn 中 admit queue → 真 `post_interrupt` → drain 退出后 watcher 不复活（draining 保持 false、LLM 恰 1 次调用、queue 行仍 pending 待下次消费） |
