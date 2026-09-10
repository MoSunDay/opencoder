Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# Agent Harness — opencode/codex 双执行器与资源快照

## 关键路径

- crates/core/src/harness/mod.rs — Harness 枚举 {opencode, codex}
- crates/core/src/harness/settings.rs — CodexSettings，Debug 不泄值
- crates/core/src/harness/runtime.rs — pin_agent_settings/agent_settings
- crates/local/src/lib.rs — CLI --wrap/--cmd/--envs
- crates/session/src/harness/mod.rs — 会话启动即固定 harness
- crates/session/src/harness/resources.rs — 工作区快照 .opencoder/runtime
- crates/session/src/harness/codex/process.rs — codex exec/resume/fork
- crates/session/src/harness/codex/decode.rs — exec JSONL 转消息
- crates/control/src/api/settings/ — 默认/命名 Codex 配置与修订号
- crates/web/spa/src/harness/ — Harness/Runner 管理页
- crates/worker/src/workloads/agent.rs — Agent 执行负载
- scripts/acceptance/harness/codex.js — 浏览器验收脚本

## 边界

- Codex 复用节点自身认证与权限，不用 OpenCoder 凭据
- 配置受理时快照；修改只影响后续新任务
- 环境变量存私有定义库，公开详情不返回
- 意外退出不自动重发已提交需求

## 相关

- [agents/session](../../agents/session/index.md)
- [agents/local](../../agents/local/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/worker](../../agents/worker/index.md)
- [注册 Runner](../../docs/registered-runners.md)
