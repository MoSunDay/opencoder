Commit: (working-tree, pre-initial-commit)

# subagent 超时语义：墙钟总时长 → step 级空闲超时

## 背景

此前 `task` 工具的超时是**单次墙钟总时长**：从子 agent 启动起算，默认 1800s，
整个生命周期只触发一次、从不重置。这意味着一个长时间运行但持续有进展的子 agent
（如连续做十几次探索/工具调用）会被误杀；而真正"卡死"的判定粒度过粗（必须整体跑满
30 分钟）。本次改为 **step 级空闲超时（idle/stall timeout）**：子 agent 每产生一次
推进活动就重置计时器，仅当某一步持续无响应超过阈值时才超时。

## 变更

### Phase-1 select! 改为 resettable loop（核心接缝）
- **`crates/session/src/runner/execute.rs`**（`execute_call_with_timeout` 的 `task` 分支）：
  - 新建 `mpsc::channel::<()>(16)` 活动信号通道；`act_tx` 传入 `run_subagent`。
  - 单次 `select!` → `loop { select! }`，新增 activity 分支：收到信号即
    `deadline.as_mut().reset(Instant::now() + task_dur)`。
  - **biased 顺序** cancel > activity > timeout > sub：持续活动优先 reset，
    避免边界处误判超时。
  - **closed-channel 守卫**：用 `if activity_alive` 前置条件禁用 activity 分支。
    sender 在子 agent run_loop 返回时 drop（之后仅 flush + DB 写入），此时 receiver
    会无限返回 `None`；若不加守卫，biased select 会让 `None` 永远抢赢、饿死 `sub`
    future（子 agent 永远无法 resolve → 死等）。`None` 时置 `activity_alive = false`，
    后续仅由 deadline / sub / cancel 决出胜负。

### 回调内发活动信号
- **`crates/session/src/runner/subagent.rs`**（`run_subagent`）：
  - 签名新增 `activity: tokio::sync::mpsc::Sender<()>`。
  - 事件回调闭包顶部对所有 `SessionEvent` 统一 `activity.try_send(())`（非阻塞，
    满/关闭静默丢弃——信号幂等，最近一次真实活动起算即可）。覆盖 ToolStart / ToolEnd、
    TextDelta / ReasoningDelta，避免 LLM 正常长生成阶段被误判超时。

### 配置文档语义对齐
- **`crates/core/src/config.rs`**：`task_timeout_secs` 字段注释 + `task_timeout()`
  访问器注释从 "Max wall-clock duration" 改为 "Per-step idle timeout"（每次活动即重置，
  仅卡住无响应超此时长才超时）。默认值 1800s 不变，访问器签名不变。

### Phase-2 宽限 drain 不动
- execute.rs 的 grace-drain（Phase 2）与超时触发方式无关，保持原样。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 持续活动不超时（总时长 > timeout 但每步间隔 < timeout） | `sustained_activity_does_not_timeout` | `crates/session/tests/subagent_timeout_cancel.rs` |
| 卡住单步超时（单次 bash 无中间事件，~timeout 后 Cancelled） | `stalled_single_step_times_out` | `crates/session/tests/subagent_timeout_cancel.rs` |
| 超时后 DB 任务 Cancelled（既有，回归守卫） | `timeout_marks_subagent_cancelled` | `crates/session/tests/subagent_timeout_cancel.rs` |

- `sustained_activity_does_not_timeout` 是**关键回归点**：在旧（墙钟总时长）语义下此用例
  会在 1s 处被杀；新语义下 6 次 `sleep 0.2`（总 ~1.2s > 1s）正常完成（Completed）。
- `cargo test -p opencoder-session` → 除一条 **预先存在**的 `control_cmd::queue_drains_*`
  失败（来自工作树既有的 mod.rs queue-drain 重构，与本变更无关）外全部通过。
- `cargo test -p opencoder-core` → 全部通过。
- clippy（execute.rs / subagent.rs / config.rs）：零新告警。

## Impact Surface
- **行为变更**：`task_timeout_secs` 含义从"总执行时长上限"变为"单步空闲超时"。
  长时间运行但活跃的子 agent 不再被误杀；仅真正卡死（某步无响应超阈值）才超时。
- 已配置该值的用户需知悉语义变化（默认 1800s 不变，仅解读方式改变）。
- 接缝不变：`Store` / `ChatStream` 抽象、Phase-2 drain、cancel 语义均未改动。

## Related Docs
- [agents/session](../../agents/session/index.md) — drain 主循环、subagent 调度
