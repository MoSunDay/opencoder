Commit: (working-tree, pre-initial-commit)

# fix(tui): 自愈 UI 通道饱和丢掉的 lifecycle 事件 + 子 agent 运行态门控

## 背景

session → TUI 的事件转发经 `forward_event`，其非 delta lifecycle 事件用
lossy `try_send`。当 UI 渲染跟不上、通道饱和时，关键 lifecycle 事件会被丢弃：

1. **`LlmRoundStart` 被丢** → `llm_round_started_at_ms` 停留 `None`，turn 计时器
   永不启动，首个 `TextDelta`/`ReasoningDelta` 落在一个「无锚点」的 round 上。
2. **`SubagentEnd` 被丢** → 某个 subagent 块永远停留在 spinning 态，turn 结束后
   仍显示「运行中」，且 `subagents_running` 在 `Done` 已清零、`running` 为 false，
   但幽灵块仍在——此时 mode-switch running-gate 会**误判为 idle**，让 Shift+Tab /
   `/act` / `/plan` 在一个仍被追踪的子 agent 存活时静默切模式（在任意半截边界完成
   「切换」，下一 turn 以 stale agent 启动）。

本次以「自愈 + 主动门控」两层兜底：在不可避免的「首个 delta」与「turn 权威 idle
边界」处补锚/收尾；同时让 mode-switch 门控把 `subagents_running > 0` 也视为 busy，
使丢事件不再留下可见残留态、也不会让切模式溜过。

## 变更

### 1. round 锚自愈 — `crates/tui/src/chat.rs`
- 新增 `recover_round_anchor_if_missing(&mut self)`：仅当
  `llm_round_started_at_ms` 为 `None` 时回填 `now_ms()`（幂等，不影响已有锚）。
- `SessionEvent::TextDelta` / `ReasoningDelta` arm 在 `ensure_*_open()` 前各调一次，
  首个 delta 即补锚（计时不卡）。

### 2. 孤儿子 agent 自愈 — `crates/tui/src/chat_helpers.rs`
- 新增 `impl ChatView::reconcile_orphaned_subagents(&mut self)`：遍历 blocks，将所有
  `done == false` 的 `Subagent` 块标记为 done + ok=false + summary=`"(interrupted)"`，
  清空其子 view 的 round 锚与 steer 队列，并以 `now_ms() - started_at_ms` 回填
  `elapsed_ms`。镜像 resume/replay 把 stale `Running` DB 行映射为 `(interrupted)`。
- 调用点：`chat.rs` 的 `SessionEvent::Done` / `SessionEvent::Error`，以及
  `app_loop.rs` 的 `TurnDone`（阻塞 send，权威 idle 边界——`Done` 自身可能 lossy，
  但 `TurnDone` 必到，作最终兜底）。

### 3. mode-switch 门控纳入 subagents_running — `crates/tui/src/worker.rs` + `app_loop.rs`
- `gate_switch(running: bool)` → `gate_switch(busy: bool)`：参数语义从「turn 运行中」
  扩展为「busy」。调用方（`app_loop.rs` 的 `/act` / `/plan` / `/clear-context` 三处）
  传入 `*running || chat.subagents_running > 0`，使存活的子 agent 也阻止切模式。
  doc-comment 同步更新说明 busy 的构成。
- `handle_switch_agent`（Shift+Tab / Alt+Tab / Ctrl+Shift+Tab / t+Tab 入口，`app_loop.rs`）
  的内联门控 `if *running` 同步扩展为 `if *running || chat.subagents_running > 0`，
  覆盖快捷键切模式路径：斜杠命令走 `gate_switch`、快捷键走内联判断，两者一致，
  无任一入口可在存活子 agent 期间静默切模式。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `Done` 将孤儿子 agent 块标 `(interrupted)` | `done_reconciles_orphaned_subagent_blocks` | `crates/tui/src/subagent_tests.rs` |
| `Error` 将孤儿子 agent 块标 `(interrupted)` | `error_reconciles_orphaned_subagent_blocks` | `crates/tui/src/subagent_tests.rs` |
| 子 agent 存活（running=false）时切模式为 noop | `switch_while_subagent_running_is_noop_even_when_running_false` | `crates/tui/src/app_loop_tests/switch_gate_tests.rs` |
| `/act` 在子 agent 存活时为 noop | `slash_act_while_subagent_running_is_noop` | `crates/tui/src/app_loop_dispatch_cmd_tests.rs` |

- 全量回归：`cargo test --workspace` → 2220 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- 行数：`chat.rs` ≤ 800；`chat_helpers.rs` ≤ 400；`worker.rs` ≤ 400；`app_loop.rs` ≤ 800

## Impact Surface
- **修复**：UI 通道饱和丢 `LlmRoundStart`/`SubagentEnd` 时不再留下卡死计时器或
  幽灵 spinning 子 agent；mode-switch 门控在子 agent 存活时也拒绝切换。
- **不变**：不触及 session runner / store / chat 数据形状；纯 TUI 侧收尾 + 门控。
- **不影响**：CLI/Web/session/store 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
