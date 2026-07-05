Commit: (working-tree, pre-initial-commit)

# session 模块

## 职责
agent 运行时核心。驱动「接收输入 → 调 LLM → 执行工具 → 持久化」的主循环，并实现 opencode 两段式 delivery（steer/queue）、上下文压缩、会话恢复、title 生成、可中断（turn 边界 + mid-tool 硬中止）。

## 边界与非目标
- 不做 HTTP / 终端 IO（由 web/cli/tui 负责）。
- 不直接连数据库驱动——经 `Store` trait 抽象。
- 非目标：MCP / 权限确认（当前未实现）。
- skills 仅承接「可选的系统提示注入」：`SessionState.skill_prompt`（`Option<String>`）由调用方（TUI `$` 选择器）设置，每轮 `build_system` 把它作为 `## Active skill` 段追加到系统提示末尾（最高优先级）。skill 的发现/解析/选择 UI 不在本模块（见 core 的 `skill` 模块与 tui 的 `menu` 模块）。

## 关键抽象
- `SessionState`（`src/lib.rs`）：`id/messages/agent/model/config/client(Arc<dyn ChatStream>)/store(Option<Arc<dyn Store>>)/cancel(Option<CancellationToken>)/skill_prompt(Option<String>)` + `working_dir/step/last_usage/persisted_count/session_created`。`cancel` 由调用方挂载（`with_cancel`）：web 经 `POST /interrupt`、tui 经双击 Esc 触发；同一 token 在 run_loop 顶部与 mid-tool select! 两处生效。`skill_prompt` 经 `with_skill` 或运行时设置，由 `build_system` 注入系统提示。
- `record(&mut self, msg)`（`src/lib.rs`）：push 到内存 + 若有 store 则持久化（best-effort，失败仅 warn）。runner 所有 message 入口都走它。
- `run_loop`（`src/runner.rs`）：drain 主循环。每轮顶部：① cancel 检查（turn 边界，web/tui 触发）② **claim_steers**（提升 pending steer，重置 step=1，safe provider-turn boundary）③ 压缩判定 ④ 单次 LLM 调用 ⑤ 工具执行 ⑥ 若无工具调用（idle）→ **claim_one_queued**（恰好一条），有则续跑，无则 Done。硬中止：`run_one_llm_call` 的流接收循环与 `execute_call` 的工具 `.await` 均包在 `tokio::select!` 中监听 `await_cancel(session)`；触发后前者回空 turn、后者回 `interrupted` 工具结果并 break 工具循环，下一轮顶部 cancel 检查收尾。bash 工具设 `kill_on_drop(true)`，select! 取消即丢弃 future → 杀子进程。子 agent 复用父 token（`child.cancel = parent.cancel.clone()`）。doom-loop 守卫：`DOOM_THRESHOLD=3`，连续空 turn 达阈值后注入提示打破循环。
- **subagent**（`src/runner.rs::run_subagent`）：`task` 工具触发子 agent 运行；子 SessionState 用独立 agent（kind=subagent），事件经 `on_event` 转发到父级——`SubagentStart{id,kind,prompt}` / `SubagentEnd{id,ok,summary}`。子 agent 复用父的 client/store/cancel。
- `compaction::should_compact`（`src/compaction.rs`）：双信号——token 估算（首轮即可触发，无需模型回传 usage）+ 模型回传 usage；预算 = `min(context_threshold, context_limit - reserved)`，故 `reserved` 真正缩减可用窗口（不再是死字段）。摘要用 `small_model`。
- `resume::resume`（`src/resume.rs`）：从 Store 重建 SessionState，model/agent 取自存储的 session meta（忠实原配置）。`generate_title` 用 small_model 异步生成标题。

## 主流程
CLI/HTTP 入口 → 建/恢复 SessionState（store 可选）→ `run(session, prompt, on_event)` → run_loop 循环到 Done / interrupt / max_steps → （CLI）异步 generate_title。

empty prompt = drain 模式：不 push 合成 user msg，依赖 store 中已 admit 的 steer/queue 提供输入（web 用）。

## 依赖与接口
- 依赖：opencode-core、opencode-llm（ChatStream）、opencode-store（Store）、tokio-util（CancellationToken）。
- 被依赖：web（drain_to_completion）、cli（run_headless / resume）、tui。

## 相关模块
- [agents/store](../store/index.md) — 持久化与输入提升。
- [agents/llm](../llm/index.md) — ChatStream 与 MockChatClient。

## 代表性锚点
- drain 语义测试：`tests/steer_followup.rs`（steer 边界提升+step 重置、多 steer 一次重置、queue idle 恰好一条、durable pending 跨进程）
- 压缩配置测试：`tests/compaction_and_model.rs`（token 估算首轮触发、reserved 缩窗、small_model 用于摘要）
- 恢复测试：`tests/recovery.rs`（字节级历史重建、continue 取最新、fork 不污染父、跨进程）
- 硬中止测试：`tests/hard_abort.rs`（cancel 中止运行中的 bash `sleep`，sub-3s 返回 + `Status(interrupted)`；turn 边界 cancel 见 web 的 interrupt 测试）
