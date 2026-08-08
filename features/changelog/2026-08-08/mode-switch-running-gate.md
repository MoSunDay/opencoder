# fix(tui,web): mode-switch 运行中运行门（running-gate）——结构上不可能在 turn 中途切模式

## 背景

`running`/`draining` 为真时（worker 正在 `run_session`、web 正在 drain），
模式切换（plan↔act）若放行，会在**任意部分边界**开始下一轮：

- 切换动作进入 `UiCmd::SwitchAgent` 队列后，要到当前 `process_cmd(UiCmd::Prompt)`
  返回（turn 边界）才被消费——但 TUI 侧 `chat.agent` / `sys_tokens` 在队列
  送出时就已乐观更新，导致**面板状态与实际执行 agent 在一轮之内分叉**。
- plan→act 的 `SwitchAndStart`（含 `plan_submitted` 的自动 handoff）同理：
  running 中送出的切换会把「plan 模式下正在生成的计划」换成 act 的系统提示，
  当前轮的回答还在旧 prompt 下继续。

本次把「运行中不可切模式」从约定升级为**结构上不可能**：所有 4 个入口
（TUI `handle_switch_agent`、TUI `/` 菜单分发、TUI `SwitchAgentNoClear`、
Web `POST /agent`，另 `POST /model` 一致处理）都在发送/落库**之前**检查运行态。

## 变更

### TUI —— `worker.rs` + `app_loop.rs` + `app.rs`
- 新增纯函数 `worker::gate_switch(running) -> SwitchGate { Run, SkipRunning }`，
  与既有 `gate_compact` / `gate_clear_all` 同款；运行中一律拒绝。
- `app_loop::handle_switch_agent` 的 running-gate 从「仅 plan→act+已提交计划」
  放宽为**所有**切换：`*running` 为真 → no-op + `⏳ busy — switch when idle`
  flash，`sys_tokens` 不更新（保持当前 mode 基线，不污染本轮上下文仪表）。
  同一规则覆盖 plan→act(已提交/未提交) 与 act→plan。
- `handle_switch_agent` 新增 `no_handoff: bool` 参数（第 2 参，`name` 之后）：
  `true` 用于 `SwitchAgentNoClear`（t+Tab，保留全文、跳过 plan→act handoff），
  `false` 用于 Shift+Tab。两条路径都走 running-gated 处理器，杜绝
  `SwitchAgentNoClear` 直发 `UiCmd::SwitchAgent` 绕过闸门的漏点。
- `/` 菜单分发 `SlashAction::{Act, Plan, ClearContext}` 三臂改经
  `match gate_switch(*running)` 路由：`SkipRunning` 时推黄色
  `[switch] busy — retry when idle` marker（与 `/compact` 的 busy 反馈一致）。
- `worker.rs` 的 `UiCmd::SwitchAgent` / `SwitchAndStart` 臂补 defense-in-depth
  注释：单线程循环 + `run_session` 在 `process_cmd` 内同步，切换只在干净 turn
  边界被消费；app-loop running-gate 额外保证 running 时不发送。

### Web —— `api.rs`
- `POST /sessions/:id/agent`：先 get-or-create handle，再检查
  `handle.draining.load(SeqCst)` → `409 "agent switch refused while drain running"`，
  **在任何** store-meta / override 变更之前（原子性），镜像 `post_interrupt` 的
  draining-gate。
- `POST /sessions/:id/model`：同一 409 gate（`"model switch refused while drain running"`），
  `persist_default` 的 config 落盘 / session meta / override 全部在闸门之后。
- 新增 `error_409` helper（与既有 `error_404` / `error_500` 并列）。

### 决策（沿用计划默认）
- **拒绝式（refuse-with-marker）**而非 defer-to-idle：TUI 运行中切换立即反馈
  busy marker，用户自行决定重试时机。
- **不新增 worker 状态位**：依赖 app-loop 运行门 + 单线程 turn 边界不变式 +
  测试三重保障；steer/queue 在运行中保持可用（本轮不触碰）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| gate 纯函数：idle → Run | `gate_switch_runs_when_idle` | `tui/src/worker/tests.rs` |
| gate 纯函数：running → SkipRunning | `gate_switch_rejects_when_running` | `tui/src/worker/tests.rs` |
| Shift+Tab 运行中 no-op（未提交计划也拦） | `switch_while_running_is_noop_even_without_submitted_plan` | `tui/src/app_loop_tests/switch_gate_tests.rs` |
| Shift+Tab 运行中 act→plan no-op | `switch_act_to_plan_while_running_is_noop` | `tui/src/app_loop_tests/switch_gate_tests.rs` |
| SwitchAgentNoClear 运行中 no-op | `switch_no_clear_while_running_is_noop` | `tui/src/app_loop_tests/switch_gate_tests.rs` |
| SwitchAgentNoClear idle 跳过 handoff | `switch_no_clear_idle_skips_handoff` | `tui/src/app_loop_tests/switch_gate_tests.rs` |
| 运行中 no-op 不污染 sys_tokens | `plan_running_noop_does_not_corrupt_sys_tokens`（flash 改 "busy"） | `tui/src/app_loop_bugfix_tests.rs` |
| `/plan` 运行中 no-op | `slash_plan_while_running_is_noop` | `tui/src/app_loop_dispatch_cmd_tests.rs` |
| `/act` 运行中 no-op | `slash_act_while_running_is_noop` | `tui/src/app_loop_dispatch_cmd_tests.rs` |
| `/act_clear_context` 运行中 no-op | `slash_clear_context_while_running_is_noop` | `tui/src/app_loop_dispatch_cmd_tests.rs` |
| Web drain 中 agent 切换 409 + store meta / override 未动 | `switch_agent_refused_while_draining` | `web/tests/web_contract.rs` |
| Web drain 中 model 切换 409 + override 未动 | `switch_model_refused_while_draining` | `web/tests/web_contract.rs` |

## 回归

- 全量 workspace：`cargo test --workspace` → **2052 passed / 0 failed / 0 ignored**（当次实跑，EXIT=0；基线 2033 passed（`tui-top-right-model-effort.md`），净增 +19 ≥ 本轮新增 11 项测试，rules/02 回归基线 ✅）
- 其中 TUI lib：1029 passed / 0 failed（含本轮 9 项 running-gate 测试；`--test-threads=4` 直跑 `.target-gate` 二进制复核）
- web_contract：15 passed / 0 failed（含本轮 2 项 drain-gate 测试）
- session：257 passed / 0 failed（control_cmd 路径未触碰）
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 0 警告 / EXIT=0
- `cargo clippy -p opencoder-web --all-targets -- -D warnings` → 0 警告 / EXIT=0
- `cargo build --workspace` → Finished / EXIT=0

## 影响面

- `agents/tui/index.md`：模式切换状态机不变式补记（Shift+Tab / t+Tab / `/act`
  `/plan` `/act_clear_context` 运行中一律拒绝、idle 才切换）。
- `features/index.md`：TUI 能力条目补充 running-gate 说明并链接本 changelog。
