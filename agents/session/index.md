Commit: df06a3b0a177c14b53171818717eaca0e0c35e64

# session 模块

会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO；steer 打断进行中 turn、queue 等 idle。

## 索引
- [harness/codex/launch.rs](../../crates/session/src/harness/codex/launch.rs) — 解析启动入口并逐项追加 Codex 参数；每轮通过 `OPENCODER_HARNESS_CONTEXT` 传递会话、线程、输入和工作目录。脚本与 Codex 共用取消和超时管理，启动失败不回退；接口见 [启动脚本](../../docs/codex-startup-script.md)。
- `src/runner/` — drain/执行/sidecar/steer，subagent 在 `runner/subagent.rs`；`runner/local_memory/` 在成功任务结束后复制消息到无 Store 的独立会话，注入内置技能并运行记忆维护，主会话结束事件在维护完成后发出
- `src/lib.rs` 导出 `run_with_registry`；Brain 的生产激活创建无工具的 Act 会话，复用同一 agent loop 完成一次事件驱动决策。
- `src/harness/` — 前端共用的执行器准备与续会话校验；`codex/` 独立处理进程、JSONL 解码、工具事件映射和回合状态，TUI/headless/Web/Operator 均经 `SessionState` 进入该模块；重连 `error` 只显示状态，`turn.failed`、非零退出或缺少完成事件使回合失败
- [harness/codex/launch.rs](../../crates/session/src/harness/codex/launch.rs) — 外部启动入口解析、逐项命令参数与会话上下文；[process.rs](../../crates/session/src/harness/codex/process.rs) 每轮通过同一进程管理机制执行脚本，允许脚本准备 Codex PATH，失败不回退，取消覆盖准备阶段。
- `src/tools/` — 工具注册与实现
- [tools/command/](../../crates/session/src/tools/command/mod.rs) — 共享命令执行、输出与后台任务；Linux/macOS 注册 `bash`，Windows 注册 `powershell`，要求 PowerShell 7.4 及以上的 7.x 稳定版。TUI 命令与 hooks 共用 `host.rs`；自动记忆会话也按宿主选择工具。
- `src/bash_guard.rs` — plan/sidecar 只读执行门：Bash 使用 [shellguard](../shellguard/index.md)，PowerShell 使用固定 AST 检查器，检查过程不执行待判断命令。
- [process/windows.rs](../../crates/session/src/process/windows.rs) — 父进程持有 Job Object，辅助进程先等待归属确认再启动命令；取消、超时、退出清理整棵进程树，清理完成以 Job Object 的活动进程数为准。Linux 保留原有 supervisor/pidfd，macOS 使用现有 fallback。
- [harness/output.rs](../../crates/session/src/harness/output.rs) — 共享模型最终文本中的 JSON 提取，不依赖 DAG 容器运行时。
- `src/compaction/` — 上下文压缩
- [skill_resolve.rs](../../crates/session/src/skill_resolve.rs)、[runner/drain.rs](../../crates/session/src/runner/drain.rs) — `literal_mentions` 控制是否跳过 `@` 扩展；队列项的 `display_text` 同时用于消费事件和用户消息展示，模型输入可以保留执行前缀。
- `src/resume.rs` — 恢复；取消原语在 `src/lib.rs`
- `tests/` — 集成回归（`bash_guard_plan_mode.rs`、`subagent.rs`、`compaction_*` 等），需登录式环境（`HOME`/`SHELL`）
- [windows_native.rs](../../crates/session/tests/windows_native.rs) — 原生 Windows PowerShell、进程树、Codex 续会话与 junction 搜索去重；支持边界见 [Windows](../../features/windows/index.md)。
