Commit: c36ac68df313ec108549ed5b95756eb37edf5f69

# session 模块

会话运行时核心：LLM 主循环、输入 drain、工具执行与持久化。

## 关键路径

- `src/lib.rs` — `SessionState`（agent/model/client/store/cancel）；`record`/`record_checked`。
- `src/runner/entry/`、`src/runner/mod.rs` — 共享入口 `run`/`run_with_registry`；`run_loop` 主循环。
- `src/runner/steer.rs`、`src/runner/drain.rs` — `claim_steers` turn 边界全吸收；`claim_one_queued` idle 边界 FIFO 取一。
- `src/runner/registry.rs`、`src/runner/input_recovery.rs` — `build_full_registry` 装配工具；孤儿输入回收。
- `src/runner/subagent.rs`、`src/runner/sidecar.rs` — explore/build 子代理派发；`/sidecar` 快照问答。
- `src/runner/event.rs` — `SessionEvent.sse_kind/sse_data/coarse_kind`，SSE 单一真相源。
- `src/runner/llm_call.rs`、`src/runner/dedup.rs` — provider 轮生命周期；bash 超时结果去重。
- `src/control_cmd.rs` — `/act` `/plan` 纯切换、`/act_clear_context` canonical；`is_mode_control` 单一真源。
- `src/handoff.rs` — `reset_to_directive` 折叠 transcript 为执行指令。
- `src/skill_context.rs`、`src/skill_resolve.rs`、`src/skill_lifecycle.rs` — `$name` 消费时激活；正文一次性交付；run 结束清除。
- `src/bash_guard.rs` — plan 只读 bash 门 `gate`/`classify_with_dir`，cwd 严格一致。
- `src/tools/mod.rs` — 9 内建工具 `registry()`；`hide_build_subagent`。
- `src/tools/latent.rs`、`src/tools/question.rs` — latent `question`/`ssh_pty` 按 skill 解锁；`QuestionHub` 直连。
- `src/tools/bash/timeout.rs`、`src/tools/bg.rs` — `timeout/sleep N` 推 deadline；超时转后台进程。
- `src/compaction/` — `should_compact`/`exceeds_hard_limit` token 预算压缩。
- `src/resume.rs`、`src/fork.rs` — `resume`/`replay_cancelled_tasks`；`fork_session` 复制历史。
- `src/event_sink.rs` — `EventSink` 有界队列；`spawn_event_flusher`/`spawn_checked_event_flusher`。
- `src/prompt.rs`、`src/mcp/` — `build_system`+`mcp_section` 注入；MCP 客户端 `mcp__{server}__{tool}`。
- `src/process/` — 节点进程监管 pidfd/subreaper；非 Linux fail-closed。
- `src/harness/` — 执行器接缝：`initialize`、resources 快照、codex 子进程、`generate_title`。
- `src/loop_registry/`、`src/extensions/` — RAII 活跃 loop 计数；会话级注册工具。
- `src/agent_pools.rs` — 随当前 agent 切换的 tools PATH/skill roots。
- `src/subagent_steer_gate.rs` — 子代理完成前原子确认无在途 steer。
- `src/streamline.rs`、`src/mention_resolve.rs` — assistant 文本保义精简；`@path` 提及展开。
- `src/tool_guard.rs`、`src/dangling_tools.rs` — 失败阈值守卫；补未应答 tool_use 防 400。
- `src/autopilot/` — autopilot 决策/阶段/复核。

主模型、small model、子代理及标题/VERIFY 请求沿 provider 的协议路由；Responses 的 provider state 随消息持久化，resume、fork 和 compaction 保持完整工具组及续聊 output。

## 边界

- 不做 HTTP/终端 IO；持久化经 `Store`、模型经 `ChatStream` 抽象。
- steer 可打断进行中 turn；queue 必须等 idle；均先 pending 落库再提升。

## 相关

- [agents/store](../store/index.md) — 持久化与输入提升。
- [agents/llm](../llm/index.md) — ChatStream 与 MockChatClient。
- [Agent Harness](../../features/harness/index.md) — Codex/原生执行器契约。
