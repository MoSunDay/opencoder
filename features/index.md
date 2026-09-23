Commit: c60e2162be48102badf53d8b97e7cfa030b59605

# OpenCoder 能力地图 — 业务能力总索引
## 平台与编排
- [Agent 调度平台](agent-platform/index.md) — Server/Node 调度与按 ID 查执行明细。
- [DAG 工作流](../agents/dag-runtime/index.md) — agent/wasm 步骤与依赖执行，画布交互见 [docs/dag-dynamic.md](../docs/dag-dynamic.md)。
- [持久化 TODO 工作流](todos/index.md) — 父会话调度验收、独立 TODO 执行。
- 代码审查 — 显式注册的 `code-review` DAG；发布示例提供 [CLI 门禁](../scripts/platform/code_review_gate.sh)。
- [项目管理](../agents/project/index.md) — goal/milestone/todo 与多执行器。
- [版本化 Agent 与 NFS](../agents/agents/index.md) — 资源版本、共享池与只读导出。
- [大脑调度工作台](brain/index.md)、[大脑能力库](../agents/brain/index.md) — step/连线计划、分层能力调度与固定版本子计划。
- [远程管理 CLI](../agents/ctl/index.md) — 对接 Server API 与退出码契约。
- [PC 问题诊断与修复](../docs/brain-pc-issue.md) — 图文输入、源码影响面、Windows 实证、Team 构建与抓包。
## 会话与交互
- [会话运行时](../agents/session/index.md) — act/plan、压缩、subagent、恢复。
- [Agent Harness](harness/index.md) — opencode/codex 执行器与资源快照。
- [CLI](../agents/local/index.md)、[TUI](../agents/tui/index.md) — 无头运行与 Turn 阶梯交互。
- [Web 会话](../agents/web/index.md) — 流式会话、SSE 与模型发现。
## 配置与基础能力
- [配置和资源作用域](../agents/core/index.md) — 模型/压缩/命名环境与 Skill 注入。
- [模型客户端](../agents/llm/index.md)、[持久化](../agents/store/index.md) — 流式客户端与本地存储。
- 内置 Skill 工作流 — task-plan / task-plan-subagent / say-and-replay。
- [sandbox 命令分类](../agents/shellguard/index.md) — 只读模式写效应拦截。
- [测试规则](../rules/) — 功能测试、回归 gate、测试分层。
## 变更记录
- [Changelog](changelog/) — 按日期记录变化与验证。
