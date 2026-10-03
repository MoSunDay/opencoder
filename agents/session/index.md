Commit: 40dd45ed4c7c3240a0e879ac5bfec391ffb5a03c

# session 模块

会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO；steer 打断进行中 turn、queue 等 idle。

## 索引
- `src/runner/` — drain/执行/sidecar/steer，subagent 在 `runner/subagent.rs`；`runner/local_memory/` 在成功任务结束后复制消息到无 Store 的独立会话，注入内置技能并运行记忆维护，主会话结束事件在维护完成后发出
- `src/lib.rs` 导出 `run_with_registry`；Brain 的生产激活创建无工具的 Act 会话，复用同一 agent loop 完成一次事件驱动决策。
- `src/harness/` — 前端共用的执行器准备与续会话校验；`codex/` 独立处理进程、JSONL 解码、工具事件映射和回合状态，TUI/headless/Web/Operator 均经 `SessionState` 进入该模块；重连 `error` 只显示状态，`turn.failed`、非零退出或缺少完成事件使回合失败
- `src/tools/` — 工具注册与实现
- `src/bash_guard.rs` — plan/sidecar 只读 bash 门（薄适配 shellguard，fail-closed 见 [shellguard](../shellguard/index.md)）
- `src/compaction/` — 上下文压缩
- [skill_resolve.rs](../../crates/session/src/skill_resolve.rs)、[runner/drain.rs](../../crates/session/src/runner/drain.rs) — `literal_mentions` 控制是否跳过 `@` 扩展；队列项的 `display_text` 同时用于消费事件和用户消息展示，模型输入可以保留执行前缀。
- `src/resume.rs` — 恢复；取消原语在 `src/lib.rs`
- `tests/` — 集成回归（`bash_guard_plan_mode.rs`、`subagent.rs`、`compaction_*` 等），需登录式环境（`HOME`/`SHELL`）
