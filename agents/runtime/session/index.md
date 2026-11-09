Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# session 模块

会话运行时：drain 循环、工具注册、subagent、压缩、resume、cancel。
接缝：只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO。

## 索引
- [src/lib.rs](../../../crates/session/src/lib.rs) — 模块根与公共导出
  （`run`、`run_with_registry`）
- [src/runner/](../../../crates/session/src/runner) — drain/执行/sidecar/steer；
  [subagent.rs](../../../crates/session/src/runner/subagent.rs) 子代理、
  [local_memory/](../../../crates/session/src/runner/local_memory) 记忆维护、
  [entry/](../../../crates/session/src/runner/entry/mod.rs) 入口 future、
  [drain.rs](../../../crates/session/src/runner/drain.rs) 队列消费
- [src/harness/](../../../crates/session/src/harness) — 前端共用的执行器准备与
  续会话校验（`SessionState` 入口）
- [codex/launch.rs](../../../crates/session/src/harness/codex/launch.rs)、
  [process.rs](../../../crates/session/src/harness/codex/process.rs) — Codex
  进程启动与管理
- [codex/control/](../../../crates/session/src/harness/codex/control/mod.rs)
  — 可选外部输入协议
- [harness/output.rs](../../../crates/session/src/harness/output.rs) — 共享模型
  最终文本的 JSON 提取
- [src/tools/](../../../crates/session/src/tools) — 工具注册与实现；
  [command/](../../../crates/session/src/tools/command/mod.rs) 共享命令执行与
  后台任务
- [src/bash_guard.rs](../../../crates/session/src/bash_guard.rs) — plan/sidecar
  只读执行门，Bash 判定走 [shellguard](../shellguard/index.md)
- [src/compaction/](../../../crates/session/src/compaction) — 上下文压缩；
  `SUMMARY_PREFIX` 是合成摘要消息的公开标记（前端回放据此渲染压缩内容）
- [src/prompt.rs](../../../crates/session/src/prompt.rs)、
  [prompt/operator.rs](../../../crates/session/src/prompt/operator.rs) — 系统提示
  构建与 Operator 规则
- [skill_resolve.rs](../../../crates/session/src/skill_resolve.rs) —
  技能与 `@` 提及解析
- [src/resume.rs](../../../crates/session/src/resume.rs) — 会话恢复；
  [src/fork.rs](../../../crates/session/src/fork.rs) — 会话派生
- [src/process/](../../../crates/session/src/process) — 进程身份与平台进程树
  （[identity.rs](../../../crates/session/src/process/identity.rs)、
  [windows.rs](../../../crates/session/src/process/windows.rs)）
- [tests/](../../../crates/session/tests) — 集成回归；支持边界见
  [Windows](../../../features/platform/windows/index.md)
