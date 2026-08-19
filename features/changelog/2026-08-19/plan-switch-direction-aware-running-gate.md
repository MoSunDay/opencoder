Commit: (working-tree, post-7a9f188)

# 模式切换 running 门双向拦截（收回方向感知放行）

## 背景

7d3c056 把 Tab 键路径的 running 门改成「方向感知」：plan→act 运行中拦截（busy 提示），act→plan 运行中放行为**纯状态切换**（乐观 `fold_agent_switch` + `UiCmd::SwitchAgent` 入队，worker 单线程在 turn 边界消费）。这个前提错了：

- 产品契约是「**只有非 running 才能切模式，双向拦截**」。slash 路径（`/act` `/plan` `/act_clear_context` → `dispatch_mode_switch` → `worker::gate_switch`）从未放过行——运行中一律拒绝（`[switch] busy — retry when idle`）。方向感知门让键路径与 slash 路径**契约分叉**：同一产品动作按入口不同而行为不同。
- 「worker 在 turn 边界消费、`sess.agent` 不会中途翻转」的论证只覆盖 worker 侧；UI 侧仍会在运行中乐观折叠（chip 翻转、`sys_tokens` 刷成下一模式的基线、stale `plan_submitted` 收敛），面板状态与实际执行 agent 在一轮之内分叉——这正是 [mode-switch running-gate](../2026-08-08/mode-switch-running-gate.md) 要结构性排除的「mid-stream fold 撕裂」。
- 运行中放行还引入了一条「turn 进行中 UI 已切到 plan、transcript 已按 plan 语义折叠」的窗口：若随后 turn 以 Error/Done 收口或用户立刻提交，UI 状态机与 worker 状态机的相位差没有任何对账点。

## 根因

- `app_loop::handle_switch_agent` 的门条件从 `*running || subagents_running > 0` 收窄为 `plan_to_act && (...)`——把「act→plan 只是纯状态切换」当成等价安全的依据，但契约层面两条方向都必须在 idle 边界发生。
- `worker::gate_switch` 文档与 `UiCmd::SwitchAgent` 臂的 DEFENSE-IN-DEPTH 注释同步改写为「该 gate 仅服务 slash 路径 / act→plan MAY be enqueued mid-turn」，固化了分叉叙事。

## 变更

- **`crates/tui/src/app_loop.rs`（`handle_switch_agent`）**：门改回**双向拦截**——`if *running || chat.subagents_running > 0` 即拦截（不看方向）；busy flash 改为方向中性文案 `⏳ busy — mode switch blocked, retry when idle`（刻意不含 "plan" 子串，避免 render.rs `contains("plan")` 的 mode-flash 芯片误染为 plan 色）；拦截时无 cmd 发送、`sys_tokens`/input/cursor/running/agent 全部原样，仅 flash 反馈。**保留**：`sys_tokens_for` 刷新、`fold_agent_switch` 乐观折叠（含双击卫生：折叠同步收敛 stale `plan_submitted`）、idle 时 plan→act 的 `SwitchAndStart` 交接分支、`no_handoff`（t+Tab）语义、`SwitchOutcome::Quit`（worker 死亡）处理。doc 注释重写为双向拦截契约，与 slash 路径 `worker::gate_switch` 对齐（净 -3 行，794/800）。
- **`crates/tui/src/worker.rs`**：`gate_switch` doc 删除「该 gate 现仅服务 slash 路径 / 键路径用方向感知门」表述，改为双向拦截统一契约（slash 与键路径同门）；`UiCmd::SwitchAgent` 臂 DEFENSE-IN-DEPTH 注释删除「act→plan MAY be enqueued mid-turn」，恢复「app-loop running 门双向拒绝发送，此臂只在干净 turn 边界可达」。
- **`crates/tui/src/key_handler.rs`**：subagent-focus（`input_disabled`）分支**保留**三个模式切换绑定放行（switch_mode_clear / switch_mode_keep / raw BackTab——「离开视图」白名单仍有效），仅注释更新为双向拦截表述。
- **`crates/tui/tests/`**：`switch_blocked_while_running.rs` 的共享 harness（`spawn_worker` / `wait_for_calls` / `wait_for_events` / mock 构造器）拆出到 `switch_blocked_harness/mod.rs`（91 行）——act→plan 集成测试改写后契约文件保持 ≤400 行。
- **`agents/tui/index.md`**：运行中 running-gate 段落同步回双向拦截语义。

## 测试覆盖（先红后绿）

| 功能 | 测试名 | 文件 | 断言要点 |
|------|--------|------|----------|
| act→plan 运行中拦截（改写，原 `switch_act_to_plan_while_running_switches_state_only`） | `switch_act_to_plan_while_running_is_noop` | `crates/tui/src/app_loop_tests/switch_gate_tests.rs` | 无 cmd 发送、running 保持 true、agent 留 act、input/cursor/sys_tokens(哨兵 7) 原样、flash 含 "busy"+"blocked" |
| act→plan 运行中拦截（t+Tab，改写，原 `..._no_clear_while_running_switches_state_only`） | `switch_act_to_plan_no_clear_while_running_is_noop` | 同上 | 同上（no_handoff 不绕门；sys_tokens 哨兵 11） |
| act→plan 存活 subagent 拦截（改写，原 `..._while_subagent_live_pure_switches`） | `switch_act_to_plan_while_subagent_live_is_noop` | 同上 | running=false + subagents_running=1 仍拦：无 cmd、agent 留 act、sys_tokens(哨兵 42)/input 原样、flash busy/blocked |
| plan→act 运行中拦截（保留锚） | `switch_while_running_is_noop_even_without_submitted_plan` / `switch_no_clear_while_running_is_noop` / `switch_while_subagent_running_is_noop_even_when_running_false` / `switch_plan_to_act_while_running_is_noop`（mod.rs） | 同上 + `app_loop_tests/mod.rs` | 无 cmd、agent 留 plan、sys_tokens/input 原样、flash busy/blocked（断言文案随新文案中性化） |
| sys_tokens 不被污染（保留锚，文案断言更新） | `plan_running_noop_does_not_corrupt_sys_tokens` | `crates/tui/src/app_loop_bugfix_tests.rs` | 拦截时 `sys_tokens` 保持 plan 基线哨兵；flash 含 "busy"（tick 对齐）+"mode switch blocked" |
| idle 双击回归（保留锚） | `switch_act_to_plan_collapses_stale_plan_submitted_synchronously` / `shift_tab_double_tap_second_strike_is_pure_switch_and_keeps_input` | `switch_gate_tests.rs` | tap1 同步收敛 stale `plan_submitted`；tap2 纯 `SwitchAgent`×2、不排空 input、不启 turn |
| idle 交接 / no_handoff（保留锚） | `switch_plan_to_act_while_idle_triggers_handoff` / `switch_no_clear_idle_skips_handoff` / `switch_plan_to_act_unsubmitted_is_pure_switch` | `app_loop_tests/mod.rs` + `switch_gate_tests.rs` | idle 语义不变：`SwitchAndStart` 交接 / no_handoff 不启 turn 不排空 |
| 集成：act turn 运行中按键（改写，原 `act_to_plan_pure_switch_consumed_at_turn_boundary`） | `act_backtab_blocked_while_running_then_switch_after_idle` | `crates/tui/tests/switch_blocked_while_running.rs` | 挂起 act turn 中无 AgentSwitch、恰 1 次 LLM 调用；TurnDone(act) 后 idle 重按 → TurnDone(act) 严格先于 AgentSwitch(plan)、agent 内存+store 翻转 plan、plan phase 重置、仍恰 1 次调用 |
| 集成：plan turn 运行中按键（保留锚） | `plan_backtab_blocked_while_running_then_handoff_after_idle` | 同上 | 拦截窗口无 AgentSwitch/TranscriptReset/PlanHandoff；idle 重按 → 完整 handoff（折叠单条计划消息 + PlanHandoff 卡片 + 恰 2 次调用） |
| key handler 层（保留锚，仅注释更新） | `handle_key_disabled_allows_backtab_mode_switch` / `handle_key_disabled_backtab_skips_plan_compound_submit` / `handle_key_disabled_allows_bound_mode_switch_keys` | `crates/tui/src/key_handler_disabled_mode_tests.rs` | `input_disabled` 下三绑定仍返回 `KeyAction::SwitchAgent(_)/None`——门在 `handle_switch_agent`，不在 key handler |

- 红（改测试、实现未动）：`cargo test -p opencoder-tui --lib -- switch_gate` → 6 passed / **3 failed**（`switch_act_to_plan_while_running_is_noop` / `switch_act_to_plan_no_clear_while_running_is_noop` / `switch_act_to_plan_while_subagent_live_is_noop`，均 panic 于 "no command should be sent ..."——旧实现入队了纯切换）。
- 绿（改实现后）：`switch_gate` 9 passed / 0 failed；`plan_running_noop` 1 passed；`--test switch_blocked_while_running` 2 passed / 0 failed。
- 全量回归：`cargo test -p opencoder-tui` → **1518 passed / 0 failed**（1448 lib + 70 integration，24 个测试二进制）；`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 0 警告 / EXIT=0。
- 行数：`app_loop.rs` 794 / 800（净 -3）；`switch_gate_tests.rs` 580 / 800；`switch_blocked_while_running.rs` 351 + 拆出的共享 harness `switch_blocked_harness/mod.rs` 91（均 ≤400，harness 拆分保持契约文件在新文件上限内）。

## Impact Surface

- TUI 用户：**任何方向**的模式切换（Shift+Tab / t+Tab / Alt+Tab / Ctrl+Shift+Tab）在 turn 运行中或 subagent 存活时统一收到 `⏳ busy — mode switch blocked, retry when idle` 提示，无 cmd 发送、无自动补发；idle 边界重按即按 idle 语义执行（plan→act 已提交计划 → 交接；否则纯切换）。act turn 运行中「立即切到 plan」的行为收回。
- **mid-stream fold 撕裂路径随之不可达**：运行中不再乐观折叠/不再入队 `SwitchAgent`，UI（chip/sys_tokens/transcript 折叠相位）与 worker 执行 agent 在一轮内分叉的窗口被结构性关闭；`UiCmd::SwitchAgent` 臂只在干净 turn 边界可达（DEFENSE-IN-DEPTH 注释恢复）。
- 不影响：slash 路径（`/act` `/plan` `/act_clear_context` → `worker::gate_switch`）本就双向拒绝，现与键路径同契约；subagent-focus 视图三个模式切换键绑定仍放行（裁决仍在 `handle_switch_agent`）；web `POST /agent` drain 中 409 不变；worker `SwitchAgent`/`SwitchAndStart` 臂行为不变（仅注释/契约更新）。

## Related Docs

- [2026-08-08/mode-switch-running-gate.md](../2026-08-08/mode-switch-running-gate.md)（双向拦截的原始契约，本轮收回后重新生效）
- `agents/tui/index.md`（运行门/key_handler 段落同步双向拦截语义）
