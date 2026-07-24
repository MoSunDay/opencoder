Commit: (working-tree, pre-initial-commit)

# fix(tui): plan→act handoff 延迟到 turn 结束 + 运行计时器 + workdir 标题

## 背景

commit `db71fa1` 把 `run_app` 主循环中三块内联逻辑抽成纯/异步函数
（`compute_display` / `handle_switch_agent` / `fold_ui_events` / `tick_clock`），
以便在新建的 `app_loop_tests.rs` 里单测。抽取同时修掉三个真实缺陷——这些缺陷
在该提交信息里未记录、亦无 changelog 覆盖：

1. **P0 竞态（plan→act while running）**：plan 模式提交过计划后、在 turn 仍在跑时
   切 act，handoff 此前会**立即**触发。当前 turn 尚未结束，`SwitchAndStart` 与正在
   进行的 drain 互相打架，导致计划被错误截断 / 重复执行。
2. **P1 `plan_submitted` 被压缩清零**：`TranscriptReset`（replay / 压缩）通过
   `..Default::default()` 重建 `ChatView`，把 2026-07-23 引入的 `plan_submitted` 标志
   清成 `false`；于是压缩后切 act 不再触发 handoff，计划被当普通对话丢弃。
3. **缺少运行计时器**：状态栏只显示静态 status 文本，看不出当前 turn 跑了多久。

## 变更

### `crates/tui/src/app_loop.rs`（新文件，从 `app.rs` 抽取）
- **`handle_switch_agent`**：plan→act 且 `plan_submitted` 时——
  - **idle**：立即 handoff（`SwitchAndStart`），flash `→ act mode`。
  - **running（P0 修复）**：把输入框残余文本存入 `pending_handoff`，flash
    `→ act (pending)`，**不打断**正在跑的 turn。
  - 非 plan→act 或未提交：清空 `pending_handoff`，plain `SwitchAgent`，
    flash `→ {name} mode`。
- **`fold_ui_events`**：
  - `TurnDone`（P0 修复）：若 `pending_handoff` 有值且 turn **正常结束**，`take()` 后
    触发 `SwitchAndStart("act", extra)`；若 turn 是 `Cancelled`，**不**触发 handoff
    （用户已取消，不应自动执行计划）。
  - `TranscriptReset`（P1 修复）：replay 前保存 `plan_submitted`，replay 后写回，
    压缩不再清零。
- **`tick_clock(running, last_clock, run_elapsed_ms)`**：每次循环迭代累加 wall-clock
  毫秒，仅当 `running` 时计入（`saturating_add`）。
- **`compute_display`**：纯函数计算每帧显示态。非 subagent-focus 时 body 标题取
  `workdir.display()`；subagent-focus 时换入子视图 + 回退标题
  （subagent 视图那条线另见 `tui-disable-input-subagent-view.md`）。

### `crates/tui/src/render.rs`
- **`format_run_duration(ms)`**：`0s / 1s / 59s / 1m / 2m / 59m / 1h0m` 格式化
  （render.rs:30）。
- 状态栏在 `run_ms > 0` 时渲染计时（render.rs:643），为 0 时隐藏。

### `crates/tui/src/app.rs`
- 主循环改为调用上述抽取函数；新增 `last_clock` / `run_elapsed_ms` 本地态喂给
  `tick_clock`。

### `crates/tui/src/app_loop_tests.rs`（新文件）
- 迁入既有 `route_paste_*` 3 个测试（行为不变，仅为满足 800 行上限而随抽取外置）。

## 设计要点
- `pending_handoff` 是纯 UI 运行态，不持久化；`Cancelled` 显式**不**消费它，避免
  取消后误执行。
- `plan_submitted` 跨 `TranscriptReset` 保活是局部补丁（仅在该事件分支 save / restore），
  不改变其「纯 TUI 状态、不落 store」的性质。
- `tick_clock` 用 `Instant` 累加，不依赖 wall-clock 绝对值，故计时测试可用固定字面量断言。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| running 时 plan→act 延迟 + flash `→ act (pending)` | `switch_plan_to_act_while_running_defers_handoff` | app_loop_tests.rs |
| idle 时 plan→act 立即 handoff | `switch_plan_to_act_while_idle_triggers_handoff` | app_loop_tests.rs |
| 非 plan→act 清空 pending | `switch_non_plan_to_act_clears_pending` | app_loop_tests.rs |
| 未提交 = 纯切换（不 handoff） | `switch_plan_to_act_unsubmitted_is_pure_switch` | app_loop_tests.rs |
| TurnDone + pending 触发 handoff | `fold_turndone_with_pending_triggers_handoff` | app_loop_tests.rs |
| Cancelled 阻断 handoff | `fold_turndone_cancelled_blocks_handoff` | app_loop_tests.rs |
| TranscriptReset 保活 plan_submitted | `fold_transcript_reset_preserves_plan_submitted` | app_loop_tests.rs |
| 计时格式化（0s..1h0m） | `format_run_duration_formats_correctly` | render_tests.rs |
| running 时状态栏显示计时 | `status_bar_shows_run_duration` | render_tests.rs |
| 计时为 0 时隐藏（渲染门） | `status_bar_hides_duration_when_zero` | render_tests.rs |

- 全量回归：`cargo test --workspace` → 921 passed; 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- 行数：app.rs 793（HEAD，已提交态）/ app_loop.rs 547 / app_loop_tests.rs 449 /
  render.rs 719（均 ≤ 800；render_tests.rs 1092 为既有超限测试文件，非本次引入）

## Impact Surface
- 仅影响 TUI 主循环与状态栏渲染；不触碰 session runner / store / llm / web / cli 的契约。
- 用户可感知：plan 跑到一半切 act 不再打架（延迟到该 turn 跑完）；压缩后切 act 仍能
  handoff；状态栏多一个运行计时；body 标题显示当前 workdir。

## Related Docs
- [既有 changelog：plan→act handoff submit guard](../2026-07-23/plan-act-handoff-submit-guard.md)
- [既有 changelog：handoff 回归测试](../2026-07-23/plan-act-handoff-regression-tests.md)
