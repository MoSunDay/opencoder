Commit: 3542e74a91260499233794c755a6a0ca5c1c8992

> 语义修正（2026-08-24，见 [hard-cancel 守卫](../2026-08-24/hard-cancel-guards-drain-steer-apply.md)）：「应用点无条件、无运行时守卫」仅对 turn 内竞态成立；外部异步 hard cancel 与 queue/steer 应用点之间存在竞态，已由后续变更在事件 emit 之前加守卫（cancel 时 unpromote 保留 pending，下次显式提交再应用）。无 cancel 时本契约全部条款不变。

# 延迟模式命令 admission：运行中文本 /plan|/act 由提交时拒绝改为边界生效

## 背景与根因

`running-mode-gate` 契约（52622d7）要求 running/draining 时所有 act/plan transition 一律提交时拒绝。但文本模式命令（`/plan`、`/act`、`/act_clear_context`，含复合内容）本质是普通 prompt：runner 在 idle/turn 边界应用控制命令时**无在途 turn**，不存在“切换落地在 turn 中途”的竞态（turn 内竞态结论不变；外部异步 hard cancel 的竞态由 2026-08-24 守卫修复，见上）。提交时拒绝的唯一收益是提前反馈，代价是运行中无法排队模式命令——与普通 prompt 的 queue/steer 语义不一致，也让 TUI 在 busy 时只能阻塞输入。

真正的模式切换风险来自**改写会话配置的入口**（`agent` 字段、POST /agent、POST /handoff、TUI 直接切换键、`/` popup、subagent steer），它们才需要 admission-time 拒绝。

## 新稳定契约

- `PromptBody.agent`、POST `/agent`、POST `/handoff`、TUI Shift+Tab 等直接切换键、`/` popup：running/draining 时仍 409 / `ModeSwitchBlocked`，无副作用。
- 文本模式命令：TUI/Web 在运行中照常 admit——Enter → steer（turn 边界应用）、Tab → queue（idle 边界应用）、BackTab → submit、web POST /prompt 任意 delivery → 200。应用点沿用 drain/steer 集成点；唯一例外守卫：应用点已取消（hard cancel）时不消费、unpromote 保留 pending（2026-08-24 修正）。
- TUI `ModeSwitchBlocked` 唯一保留分支：**聚焦运行中 subagent 视图的 Enter**（subagent 无模式概念）；subagent 聚焦下 Tab/BackTab → `QueueUnsupported`。
- subagent steer（web + TUI）仍拒绝模式命令。
- `is_mode_control` 语义不变（分类用纯判定），仅不再作为公共入口的提交时拒绝依据。

## Validation

- Session：lib 1512 + drain/queue 强化用例（`drain_one_queued_bare_control_cmd_returns_control_cmd` 断言 agent/AgentSwitch/持久化）+ `steered_control_cmd_not_recorded_as_user_text` 强化（无 TextDelta、零 LLM 调用）+ 新增 `steered_compound_plan_switches_then_runs_rest`（`/plan review` steer → plan + 单次 LLM + 无泄漏）：全部通过，0 失败。
- TUI：`key_handler_running_mode_tests` 重写为 6 用例（Enter→steer、Tab→queue、BackTab→submit、subagent Enter→ModeSwitchBlocked 且输入保留、subagent Tab→QueueUnsupported、普通文本不变）；既有 `switch_blocked_while_running` 不变通过。
- Web：`running_mode_gate` 重写 3 用例（agent 字段/专用切换路径 409 且无副作用；queue 模式命令 200 + idle 边界生效；steer 模式命令 200 + turn 边界生效，含 done 后 draining 复位）；`store_error_surfacing` 3 用例恢复通过（修复 c051030 引入的 “persist skill” 文案回归）。
- 根目录真实二进制 `running_mode_switch_e2e` 按新契约更新：运行中 POST /agent、/handoff 409；queue `/plan later` + skill 200（skill 落盘、meta 不变、messages 无泄漏）；释放阻塞 provider 后 idle 边界应用成功切 plan；`client_server_smoke` 通过。
- 全仓按包分片全量回归：core/llm/store、client/cli、todos、session/tui/web/opencoder 合计 0 失败；clippy `-D warnings`（tui/session/web/opencoder）与 `cargo fmt --all -- --check`、`git diff --check` 通过。

## 兼容性与边界

- 无数据库 schema、配置项或环境变量变化。
- 专用切换入口的 409 契约与 TUI 直接切换键行为不变；SPA 前端 busy 时的本地拦截保持不变（客户端 UX 守卫，服务端已放行文本模式命令）。
- 运行中拒绝（agent 字段等）仍不消费用户输入、不激活 skill；文本模式命令的 skill 在消费边界激活（沿用 $skill 延迟语义）。

## 相关文档

- [session 模块](../../../agents/session/index.md)
- [TUI 模块](../../../agents/tui/index.md)
- [Web 模块](../../../agents/web/index.md)
- [running 模式切换门回归加固](../2026-08-22/running-mode-gate-regression-hardening.md)（本变更修正其“文本模式命令提交时拒绝”条款）
