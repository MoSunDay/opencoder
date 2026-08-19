Commit: (working-tree, post-3320cbb)

# 模式切换 running 门改为方向感知：plan→act running 拦截+明确提示，act→plan running 纯切换，subagent-focus 视图放行模式切换键

## 背景

原 running 门（见 [mode-switch running-gate](../2026-08-08/mode-switch-running-gate.md)）对**所有方向**一刀切：`running || subagents_running > 0` 时 Shift+Tab / t+Tab 一律 no-op，flash 提示 `⏳ busy — switch when idle`。但两个方向的风险并不对称：

- **plan→act** 携带交接语义（transcript 折叠 + 立即执行），running 中执行确实不安全，一刀切有理。
- **act→plan** 只是纯状态切换（一条 `UiCmd::SwitchAgent`）。worker 单线程、`run_session` 在 `process_cmd(Prompt)` 内同步执行——running 中入队的切换必然在下一 turn 边界才被消费，在飞 turn 以旧 agent 跑完，`sess.agent` 不会中途翻转。一刀切拒绝属于过度保守：用户在长 turn 中想回 plan 写需求也被挡。

另外 subagent-focus（input_disabled）视图把 Alt+Tab / Ctrl+Shift+Tab / BackTab 全部屏蔽——用户聚焦子代理观察时无法离开/切换模式，视图状态反过来锁死了全局模式操作。

## 根因

- `app_loop::handle_switch_agent` 的门是方向无关的（`if *running || chat.subagents_running > 0` 早退），且注释/flash 都写成"延迟到 idle 边界"的含糊语义（实际从不自动补发）。
- `key_handler.rs` 的 `input_disabled` 分支只放行 quit/cancel/help/scroll，模式切换键落在兜底 `KeyAction::None`。

## 变更

- **`crates/tui/src/app_loop.rs`（`handle_switch_agent`）**：门改为方向感知——`plan_to_act && (*running || chat.subagents_running > 0)` 才拦截，flash 改为明确的 `⏳ busy — plan switch blocked, retry when idle`（无自动补发，用户 idle 后重按；sys_tokens/input/running 均不动，agent 留在 plan）。act→plan 无论是否 running 自然落到 `sys_tokens_for` 刷新 + `fold_agent_switch` 乐观折叠 + 纯 `UiCmd::SwitchAgent` 入队；plan→act idle 交接分支只在早退未触发时可达。doc 注释整体重写（删除"deferred to the next clean idle boundary"等过时表述）。
- **`crates/tui/src/worker.rs`**：`UiCmd::SwitchAgent` 臂 DEFENSE-IN-DEPTH 注释更新契约——app-loop 门现在只在 plan→act（handoff/no_handoff）方向拒绝 running 期间发送；act→plan 纯切换允许 running 中入队，依赖既有单线程"turn 边界才消费"论证。`gate_switch` doc 注明该 gate 现仅服务 slash 路径（`/act` `/plan` `/act_clear_context`），Shift+Tab 键路径用 `handle_switch_agent` 内的方向感知门（行为未变，slash 门仍双向拒绝）。
- **`crates/tui/src/key_handler.rs`（input_disabled 分支）**：`bindings.help` 之后、兜底 `None` 之前放行三个模式切换绑定（顺序 clear → keep → raw BackTab；plain BackTab 无 CTRL/ALT 修饰符不会误匹配前两个绑定）。三者都汇入 `handle_switch_agent` 由方向感知门裁决；`/plan <content>` 复合提交分支在此视图刻意跳过（输入已禁用）。

## 测试清单

| 测试 | 层级 | 断言 |
|---|---|---|
| `switch_gate_tests::switch_while_running_is_noop_even_without_submitted_plan`（改） | unit | plan→act 无 plan + running 仍拦截：无 cmd、running 保持、input/sys_tokens 不动、agent 留 plan、flash 同时含 "busy" 与 "blocked" |
| `switch_gate_tests::switch_act_to_plan_while_running_switches_state_only`（重写改名） | unit | act→plan running：恰好一条 `SwitchAgent("plan")`（绝无 SwitchAndStart）、agent 翻转、running 保持 true、input/cursor 原样、flash 含 "plan mode"、`sys_tokens == sys_tokens_for("plan", …)` |
| `switch_gate_tests::switch_no_clear_while_running_is_noop`（改） | unit | plan→act no_handoff running 仍拦截；flash 加 "blocked" 断言 |
| `switch_gate_tests::switch_while_subagent_running_is_noop_even_when_running_false`（改） | unit | plan→act + subagents_running=1 + running=false 仍拦截（doc 换方向感知表述） |
| `switch_gate_tests::switch_act_to_plan_while_subagent_live_pure_switches`（新） | unit | running=false + subagents_running=1 + act→plan：恰好一条纯切换、agent 翻转、无 turn 启动、input 保留 |
| `switch_gate_tests::switch_act_to_plan_no_clear_while_running_switches_state_only`（新） | unit | no_handoff=true + running + act→plan：纯切换，无 SwitchAndStart |
| `key_handler_disabled_mode_tests::handle_key_disabled_allows_backtab_mode_switch`（新） | unit | input_disabled 下 BackTab：plan→`SwitchAgent("act")`、act→`SwitchAgent("plan")` |
| `key_handler_disabled_mode_tests::handle_key_disabled_backtab_skips_plan_compound_submit`（新） | unit | input 预填 `/plan do the thing` + BackTab → 纯 `SwitchAgent("plan")`，input 原样（复合提交分支跳过） |
| `key_handler_disabled_mode_tests::handle_key_disabled_allows_bound_mode_switch_keys`（新） | unit | Alt+Tab（Tab/BackTab+ALT 两变体）→ `SwitchAgent`；Ctrl+Shift+Tab（BackTab+CONTROL、Tab+CONTROL\|SHIFT 两变体）→ `SwitchAgentNoClear` |
| `app_loop_bugfix_tests::plan_running_noop_does_not_corrupt_sys_tokens`（改） | unit | 行为不变（sys_tokens 不动）；doc/断言换新文案（含 "plan switch blocked"） |
| `app_loop_tests::switch_plan_to_act_while_running_is_noop`（改，doc-only） | unit | 行为不变（plan→act running 仍拦截）；doc 删除"覆盖 act→plan / 延迟 idle"的过时一刀切表述 |
| `tests/switch_blocked_while_running.rs::plan_backtab_blocked_while_running_then_handoff_after_idle`（新） | integration | 挂起 LLM call 的 plan turn 中 BackTab：无 AgentSwitch/TranscriptReset/PlanHandoff、恰 1 次 LLM 调用；TurnDone 后重按 → agent=="act"（内存+store）、transcript 折叠为单条计划消息、PlanHandoff 卡片、act 请求只见 handoff 消息 |
| `tests/switch_blocked_while_running.rs::act_to_plan_pure_switch_consumed_at_turn_boundary`（新） | integration | act turn 中入队纯 `SwitchAgent("plan")`：TurnDone(act) 严格先于 AgentSwitch(plan)、恰 1 次 LLM 调用、plan phase 重置、store agent=="plan" |
| `key_handler_tests.rs` 移除 `handle_key_disabled_blocks_alt_tab` / `handle_key_disabled_blocks_ctrl_shift_tab`（旧断言与新放行语义冲突，被上面的放行测试取代） | unit | — |

- 全量回归：`cargo test -p opencoder-tui` → 1513 passed / 0 failed（1443 unit + 70 integration）；`cargo test -p opencoder-session` 全绿。
- clippy：`cargo clippy --workspace --all-targets` → 0 warning / 0 error。
- 行数：均 ≤800（`app_loop.rs` 792 / `switch_gate_tests.rs` 581 / `key_handler_tests.rs` 694 / `app_loop_bugfix_tests.rs` 757；新文件 `key_handler_disabled_mode_tests.rs` 123、`tests/switch_blocked_while_running.rs` 383 均 ≤400）

## Impact Surface

- TUI 用户：plan turn 运行中按 Shift+Tab/t+Tab 得到明确的"plan switch blocked, retry when idle"提示（无自动补发，idle 后重按即交接）；act turn 运行中按 Shift+Tab/t+Tab 立即切到 plan（chip 即时翻转，worker 在 turn 边界应用并重置 plan phase，不启动新 turn）；聚焦运行中 subagent 视图时 Alt+Tab / Ctrl+Shift+Tab / Shift+Tab 可正常离开或切换模式。
- 不影响：slash 路径（`/act` `/plan` `/act_clear_context`）仍走 `worker::gate_switch` 双向拒绝；web `POST /agent` drain 中 409 不变；worker `SwitchAgent` 臂行为不变（仅注释/契约更新）。

## Related Docs

- [2026-08-08/mode-switch-running-gate.md](../2026-08-08/mode-switch-running-gate.md)（历史一刀切门，保留原样）
- `agents/tui/index.md`（运行门/key_handler 段落同步方向感知语义）
