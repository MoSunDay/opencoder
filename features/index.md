Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# OpenCoder 能力地图
## 平台与编排（platform/）
- [Agent 调度平台](platform/agent-platform/index.md) — Server/Node 调度与按 ID 查执行明细。
- [Ontology](platform/ontology/index.md) — 环境隔离的实体、属性、关系、图谱切面、数据保鲜与只读正文共享。
- [Windows 原生支持](platform/windows/index.md) — Windows 11 x64 的 TUI、Operator 节点与 PowerShell 7.4 及以上的 7.x 稳定版。
- [DAG 工作流](platform/dag/index.md) — Linux 二进制与 Agent 步骤、单运行共享容器和写时复制工作区。
- [持久化 TODO 工作流](platform/todos/index.md) — 父会话调度验收、独立 TODO 执行。
- [项目管理](platform/project/index.md) — 项目与专项表格、TODO 状态看板、Tag 管理，通过执行 ID 关联原生能力。
- [版本化 Agent 与 NFS](../agents/platform/agents/index.md) — 资源版本、共享池与只读导出。
- [大脑调度与运行历史](platform/brain/index.md)、[大脑能力库](../agents/orchestration/brain/index.md) — step/连线计划、分层能力调度与固定版本子计划。
- [远程管理 CLI](../agents/platform/ctl/index.md) — 对接 Server API 与退出码契约。
## 会话与交互（interaction/）
- [会话运行时](../agents/runtime/session/index.md) — act/plan、压缩、subagent、恢复。
- [Agent Harness](interaction/harness/index.md) — opencode/codex 执行器与资源快照。
- [CLI](../agents/frontend/local/index.md)、[TUI](../agents/frontend/tui/index.md) — 无头运行与 Turn 阶梯交互。
- [Web 会话](../agents/frontend/web/index.md) — 流式会话、SSE 与模型发现。
- [Responses API](interaction/responses/index.md) — Responses 协议编码工作流：provider state 持久化，resume/fork/compaction 保持完整工具组。
- [Computer use](interaction/computer-use/index.md) — 通过可选 CLI 调用 Cua Agent，操作已有远程桌面。
## 配置与基础能力（base/）
- [配置和资源作用域](../agents/foundation/core/index.md) — 模型/压缩/命名环境与 Skill 注入。
- [模型客户端](../agents/foundation/llm/index.md)、[持久化](../agents/foundation/store/index.md) — 流式客户端与本地存储。
- [本地仓库记忆](base/local-memory/index.md) — 任务完成后在独立上下文更新记忆，可在 `/config` 开关。
- 内置 Skill 工作流 — task-plan / task-plan-subagent / say-and-replay。
- [sandbox 命令分类](../agents/runtime/shellguard/index.md) — 只读模式写效应拦截。
- [测试规则](../rules/) — 功能测试、回归 gate、测试分层。
## 变更记录
- [Changelog](changelog/) — 按日期记录变化与验证。
