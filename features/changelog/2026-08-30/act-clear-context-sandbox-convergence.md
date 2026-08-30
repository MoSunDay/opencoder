Commit: (working-tree, /act_clear_context sandbox 收敛 act)

# `/act_clear_context` / Shift+Tab 在 sandbox 会话收敛到 act 模式

## 问题与根因

`ControlCmd::ClearContext` 的 apply 分支只折叠 transcript，从不切 agent（原注释钉死 "keeping the active agent"）。因此在 sandbox 会话执行 `/act_clear_context`（typed / legacy `/clear_context`）或 Shift+Tab 倒计时 fire 后，会话仍停留在 sandbox：bash 写继续被 bash_guard（shellguard）拦截，与命令名里的 `act` 语义和用户预期直接矛盾。

根因唯一：模式真值在 session 层（`SessionState.agent`），TUI 倒计时 fire 只是提交文本命令；缺的是 session 层 ClearContext 分支的「非 act → act」状态收敛。修复选在 session 层（`control_cmd::apply`），一次修改同时覆盖 idle 短路、queue drain、steer、web 文本命令 admission、CLI headless 全部入口。

## 变更

### 行为（`crates/session/src/control_cmd.rs`）
- **收敛规则**：`session.agent.kind == AgentKind::Sandbox` 时，apply ClearContext 先把 agent 换成 `resolve_agent("act")`；`persist_clear` 单次 store 写同时落边界与 `sessions.agent = "act"`（无 schema 变更，resume 自动读到 act）。
- **事件序列**：`[TranscriptReset, AgentSwitch("act")]`——AgentSwitch 条件性追加在 TranscriptReset 之后；已是 act（或其它 kind）的会话事件序列与旧版完全一致（无多余事件、无写放大），UI/重放零噪声。
- **非目标**：`/act`、`/sandbox` 语义、5s 倒计时窗口、Esc 回撤、命令命名均不变；workflow/subagent 等非 Sandbox kind 不切（guard 限定）。
- **行为推论（显式声明）**：steer 路径切到 act 后，同一 run 的后续 bash 写不再被拦——这是收敛语义的直接推论。

### 文档同步
- `ControlCmd::ClearContext` 与 `split_control_prefix` doc 改述收敛语义；TUI `app_loop_dispatch_cmd_tests/act_clear.rs` 模块 doc、`act_clear_context_fold.rs` 模块 doc 同步。

## 测试覆盖（规则 01/03，金字塔四层）

- **unit**（`crates/session/src/control_cmd.rs` 内联）：`apply_clear_context_on_sandbox_converges_to_act`——事件恰好 `[TranscriptReset, AgentSwitch(act)]`、收敛落 store 同一 patch；既有 act 会话 no-op 测试原样保留作回归。
- **integration**（新文件 `crates/session/tests/clear_context_sandbox_act.rs`，373 行）：idle 裸命令收敛+持久化+resume 仍 act、空 transcript 哨兵路径 0 次 LLM、queue drain 在真实 prompt 前收敛、steer 边界收敛、compound `/act_clear_context review` rest 在 act 下执行且命令串不泄漏。
- **单链证据**（新文件 `crates/session/tests/clear_context_sandbox_act_bash.rs`）：同一次 run 内 clear 收敛（TranscriptReset < AgentSwitch(act) < ToolEnd）后紧跟真实 bash 写（cwd 相对路径，在 sandbox 释放集之外），断言无 Block 且目录真实落地于 session cwd、收敛持久化为 act——把「收敛」与「bash 放行」两半独立证据接成一条因果链。
- **既有修正**：`crates/session/tests/control_cmd.rs` 两处 sandbox 会话测试改为断言 act 终态与 AgentSwitch；web e2e `steered_clear_context_resets_transcript_and_keeps_agent` 更名 `steered_clear_context_on_act_session_keeps_agent`，语义收敛为「already-act no-op」回归钉。
- **TUI 集成**（`crates/tui/tests/act_clear_context_fold.rs`）：`sandbox_clear_context_converges_to_act`——Worker 级断言 AgentSwitch(act) 位于 TranscriptReset 之后、seed 恰好一轮 LLM、收敛持久化。
- **web e2e**（`crates/web/tests/running_mode_gate.rs`）：`queued_clear_context_in_sandbox_session_converges_to_act`——sandbox 会话 running 期间 queue 文本命令，idle 边界 apply 后 `agent_switched` 跟随 `transcript_reset`、`meta.agent == "act"`、哨兵边界不再触发第二轮 LLM。
- **全量回归**：`cargo test --workspace` 全绿（结果见下轮 commit 记录）。

## 回滚

单语义点交付，revert 本 diff 即回滚；观察项为 web `agent_switched` 事件与 `sessions.agent` 一致性，若出现非 sandbox 会话被切立即 revert。
