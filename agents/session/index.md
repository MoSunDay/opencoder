Commit: pending

# session 模块

会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO；steer 打断进行中 turn、queue 等 idle。

## 索引
- `src/runner/` — drain/执行/sidecar/steer，subagent 在 `runner/subagent.rs`；`runner/local_memory/` 在成功任务结束后复制消息到无 Store 的独立会话，注入内置技能并运行记忆维护，主会话结束事件在维护完成后发出；维护仅在 `local-memory` 打开且任务以 act 模式运行时触发（plan/command/workflow/subagent 不触发），以 kind `memory` 子代理块回显——子会话 token 用量折入父视图成本、执行时长落在块收口，子会话产物绝不写回主会话消息
- `src/tools/` — 工具注册与实现
- `src/bash_guard.rs` — plan/sidecar 只读 bash 门（薄适配 shellguard，fail-closed 见 [shellguard](../shellguard/index.md)）
- `src/compaction/` — 上下文压缩
- `src/resume.rs` — 恢复；取消原语在 `src/lib.rs`
- `tests/` — 集成回归（`bash_guard_plan_mode.rs`、`subagent.rs`、`compaction_*` 等），需登录式环境（`HOME`/`SHELL`）
