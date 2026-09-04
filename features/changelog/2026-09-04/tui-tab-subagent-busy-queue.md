Commit: 6861975

# TUI Tab busy 判定纳入存活 subagent：idle 排空窗口不再直接提交

## Context

用户在「多个 subagent 执行中、父会话显示 idle」时按 Tab 想排队后续
指令，结果被直接 Submit 开了新 run——没有 queue 行，排队意图被背叛。

根因在 TUI 层 busy 信号不完整：`key_handler.rs` 的 Tab 分支只看父会话
`running` 布尔。而「父 `running=false` + 子代理存活」是真实窗口：
subagent 是父 turn 工具批次的内联 future，`running` 会先于批次结束翻转。

1. **autopilot 阶段间隙**（最典型）：PLAN 阶段 `Done` 到达 →
   `running=false`，随后 ACT 阶段派发多个 `task` subagent
   （`chat.subagents_running>0`）；
2. **取消宽限窗**：双 Esc 后 `cancel_running_turn` 立即置
   `running=false`，批次仍在排空（每个 task 最多 15s 宽限）；
3. **reabsorb tail**：主 run Done 后 tail one-shot 再派 subagent。

store/runner 无责：`Delivery` 过滤严格，queue 不会被误当 steer；worker
串行性保证新 run 不与 subagent 真正并发——纯粹是 UI 层 Tab 的
queue-vs-submit 臂选择错误。`chat.subagents_running`（chat_types.rs）
已存在且被 `ChatView::apply` 维护，只是从未被提交路径消费。

## Change Summary

- `key_handler.rs`：`handle_key` 签名新增 `subagents_running: bool`
  （紧邻 `running`）；Tab 分支 `if running || subagents_running →
  Queue else Submit`。
- `app.rs` 唯一调用点同行替换：`running,` →
  `running, chat.subagents_running > 0,`（该文件超 800 行上限，零净增）。
- Enter 语义不变：idle+subagent 忙时 Enter 仍是 Submit——显式提交进
  worker 串行队列，实际执行必然在 subagent 收尾后，行为正确。
- 入队后消费路径无需守卫：queue item 经同一 worker 串行执行。

## Impact Surface

- 仅 `crates/tui`：`key_handler.rs`（+参数+分支条件）、`app.rs`
  （同行实参替换）；Steer/Enter/鼠标/mode-switch 路径零变化。
- 全部 38 处 `handle_key` 调用点机械补 `false` 实参（6 个测试文件）。

## 测试清单（rules/01、02、03）

- 新增 bug 用例（unit）：
  - `key_handler_running_mode_tests.rs::
    idle_tab_with_live_subagents_becomes_queue`：
    `running=false, subagents_running=true` → Tab 返回 `Queue`；
  - `app_tests/key_tests.rs::tab_with_live_subagents_admits_queue`
    （经新 helper `run_handle_subagents_busy`）：同窗口 → `Queue`。
- 回归（既有用例继续守卫）：`tab_while_idle_submits`
  （双 idle → `Submit`）、`tab_while_running_admits_queue`
  （`running=true` → `Queue`）、
  `tab_on_focused_subagent_rejected_not_queued`
  （subagent 聚焦 → `QueueUnsupported`）。
- `cargo test -p opencoder-tui` 全量 1687 lib + 26 个集成测试二进制
  全绿；`cargo test --workspace` 其余 crate 全部通过（根包
  `daemon_smoke` 失败为同期 server 重构的既有问题，已用 stash 对照
  验证与本次无关）；clippy/fmt 对本次改动文件零新增告警。

## Related Docs

- agents/tui/index.md（key_handler 条目：Tab 双 busy 信号语义）
