# fix(session,web): late-steer 在 idle 边界被吸收，不再搁浅

## 背景

drain 主循环在每个 turn 边界顶部调用 `claim_steers` 提升待处理 steer。
但一个在 **turn 执行期间**（claim_steers 之后）被 admit 的 steer 会落入竞态窗口：
turn 结束后内层 drain 循环发现队列为空 → 发射 `Done` → `run_loop` 退出 →
该 steer 行 `promoted_seq` 保持 NULL，被搁浅，直到下一次外部请求重启 drain。

web 层虽有 defense-in-depth（drain 退出后检查 pending 并重启），但每次都
经历一次 spurious `Done` → restart 往返，造成 UI 闪烁且 steer 响应延迟。

## 变更

### 1. idle 边界 peek — `crates/session/src/runner/steer.rs`

新增纯查函数 `has_pending_steers(session) -> bool`：调用
`store.pending_inputs(sid, Delivery::Steer)` 只读检查是否有待提升 steer，
**不提升**。无 store 或读取失败时 fail-open（返回 false，正常 idle）。

### 2. drain 循环吸收 — `crates/session/src/runner/mod.rs`

内层 drain 循环在队列空时、发射 `Done` 之前调用 `has_pending_steers`：
若有待处理 steer，设置 `late_steer = true` 并 break 回外层循环，
使顶部 `claim_steers` 在下一轮将其吸收为正常 user turn（LLM 会看到它）。

### 3. web defense-in-depth 补齐 — `crates/web/src/handle.rs`

`admit_and_drain` 的 drain-restart 守卫从仅检查 `Delivery::Queue` 扩展为
同时检查 `Delivery::Steer`：即使 session 层漏掉（崩溃 / 异常退出），
web 层也会重启 drain 吸收搁浅的 steer。

## 测试清单

| 路径 | 测试 | 文件 |
|------|------|------|
| session | `late_steer_absorbed_at_idle_boundary` | `crates/session/tests/steer_followup.rs` |

测试用 `SteerOnIdle` wrapper 包裹 MockChatClient：在首个 text-only Completed
事件转发前 admit 一条 Steer，确定性地复现竞态（steer 在 idle 边界前完成 admit）。
断言：`SteerConsumed` 被发射、`LATE-STEER` 文本进入 history、pending 为空（未搁浅）。

**当次实跑**: `cargo test --workspace` → 1839 passed; 0 failed。
`cargo clippy --workspace --all-targets` → 0 warning。
