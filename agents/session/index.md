Commit: a388f28a2e631e962c01d554419cd4fc90476078

# session 模块

会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO；原生 steer 打断进行中 turn、queue 等 idle；启用外部控制协议的启动脚本通过独立控制调用投递 steer，保持主观察器运行。

## 索引
- [harness/codex/launch.rs](../../crates/session/src/harness/codex/launch.rs)、[process.rs](../../crates/session/src/harness/codex/process.rs) — 解析启动入口并逐项追加 Codex 参数；每轮通过 `OPENCODER_HARNESS_CONTEXT` 传递会话、线程、输入和工作目录。脚本可准备 Codex PATH，与 Codex 共用取消和超时管理，启动失败不回退；接口见 [启动脚本](../../docs/codex-startup-script.md)。
- [宿主代理入口](../../scripts/harness/README.md) — 普通宿主会话可复用节点已有代理运行库，每次启动或续聊建立随进程退出的桥接；Server 配置不保存临时桥接端口或代理凭据。
- `src/runner/` — drain/执行/sidecar/steer，subagent 在 `runner/subagent.rs`；`runner/local_memory/` 在成功任务结束后复制消息到无 Store 的独立会话，注入内置技能并运行记忆维护，主会话结束事件在维护完成后发出
- [runner/entry/mod.rs](../../crates/session/src/runner/entry/mod.rs) — 嵌套任务作用域及单轮、尾部处理、Autopilot、Review 的较大 future 放到堆上；普通 2 MiB Tokio 工作线程上的 Codex 首轮与恢复由 [harness_stack.rs](../../crates/session/tests/harness_stack.rs) 验证。
- `src/lib.rs` 导出 `run_with_registry`；Brain 的生产激活创建无工具的 Act 会话，复用同一 agent loop 完成一次事件驱动决策。
- `src/harness/` — 前端共用的执行器准备与续会话校验；`codex/` 独立处理进程、JSONL 解码、工具事件映射和回合状态，TUI/headless/Web/Operator 均经 `SessionState` 进入该模块；重连 `error` 只显示状态，`turn.failed`、非零退出或缺少完成事件使回合失败
- `src/tools/` — 工具注册与实现
- [tools/command/](../../crates/session/src/tools/command/mod.rs) — 共享命令执行、输出与后台任务；Linux/macOS 注册 `bash`，Windows 注册 `powershell`，要求 PowerShell 7.4 及以上的 7.x 稳定版。TUI 命令与 hooks 共用 `host.rs`；自动记忆会话也按宿主选择工具。
- `src/bash_guard.rs` — plan/sidecar 只读执行门：Bash 使用 [shellguard](../shellguard/index.md)，PowerShell 使用固定 AST 检查器，检查过程不执行待判断命令。
- - [process/identity.rs](../../crates/session/src/process/identity.rs) — 隐藏 supervisor 协议的身份参数（`--uid/--gid/--groups`，拒绝 uid/gid 0）：workload exec 前按 setgroups→setgid→setuid 固定顺序降权并收紧 umask；runc 链路不传身份，macOS fallback 接受并忽略，bash/host/bg/Codex 均透传会话 `run_as`，身份只走结构体不走环境变量。
[process/windows.rs](../../crates/session/src/process/windows.rs) — 父进程持有 Job Object，辅助进程先等待归属确认再启动命令；取消、超时、退出清理整棵进程树，清理完成以 Job Object 的活动进程数为准。Linux 保留原有 supervisor/pidfd，macOS 使用现有 fallback。
- [harness/output.rs](../../crates/session/src/harness/output.rs) — 共享模型最终文本中的 JSON 提取，不依赖 DAG 容器运行时。
- `src/compaction/` — 上下文压缩
- [prompt/operator.md](../../crates/session/src/prompt/operator.md)、[operator.rs](../../crates/session/src/prompt/operator.rs) — Operator 默认规则与目录列表纯函数；会话从持久化 `kind` 恢复执行类型。原生请求和上下文预算共用 `build_session_system`，Codex 每轮携带同一规则；目录复用 Agent 私有技能池优先的会话发现顺序。
- [skill_resolve.rs](../../crates/session/src/skill_resolve.rs)、[runner/drain.rs](../../crates/session/src/runner/drain.rs) — `literal_mentions` 控制是否跳过 `@` 扩展；队列项的 `display_text` 同时用于消费事件和用户消息展示，模型输入可以保留执行前缀。
- `src/resume.rs` — 恢复；取消原语在 `src/lib.rs`
- [fork.rs](../../crates/session/src/fork.rs) — 派生会话复制类型、消息与 Harness；指定新工作目录时更新目录标识并重建运行资源，Codex 保留来源线程引用。
- `tests/` — 集成回归（`bash_guard_plan_mode.rs`、`subagent.rs`、`compaction_*` 等），需登录式环境（`HOME`/`SHELL`）
- [windows_native.rs](../../crates/session/tests/windows_native.rs) — 原生 Windows PowerShell、进程树、Codex 续会话与 junction 搜索去重；支持边界见 [Windows](../../features/windows/index.md)。

- [harness/codex/control/](../../crates/session/src/harness/codex/control/mod.rs) — 可选外部输入协议，持久记录每条实时输入、原父输入和回执。恢复先核对外部发送结果，已转发消息不再进入普通提示词；拒绝明确展示，队列保持轮次边界。
