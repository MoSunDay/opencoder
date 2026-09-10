Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# OpenCoder 能力地图 — 业务能力总索引

## 平台与编排

- [Agent 调度平台](agent-platform/index.md) — Server/Node 调度与按 ID 查执行明细。
- [DAG 工作流](../agents/dag-runtime/index.md) — agent/wasm 步骤、依赖执行与产物。
- [持久化 TODO 工作流](todos/index.md) — 父会话调度验收、独立 TODO 执行。
- [项目管理](../agents/project/index.md) — goal/milestone/todo 与多执行器。
- [版本化 Agent 与 NFS](../agents/agents/index.md) — 引用卡、共享池、只读导出。
- [大脑能力库](../agents/brain/index.md) — 能力录入、语义检索与路由。
- [远程管理 CLI](../agents/ctl/index.md) — 对接 Server API 与退出码契约。

## 会话与交互

- [会话运行时](../agents/session/index.md) — act/plan、压缩、subagent、恢复。
- [Agent Harness](harness/index.md) — opencode/codex 执行器与资源快照。
- [CLI](../agents/local/index.md) — 无头运行、TODO 命令与工具安装。
- [TUI](../agents/tui/index.md) — Turn 阶梯、模式切换与复制。
- [Web 会话](../agents/web/index.md) — 流式会话、SSE 与模型发现。

## 配置与基础能力

- [配置和资源作用域](../agents/core/index.md) — 模型/压缩/命名环境与 Skill 注入。
- [模型客户端](../agents/llm/index.md) — OpenAI 兼容流式与测试夹具。
- [持久化](../agents/store/index.md) — 本地存储与会话/执行状态。
- [sandbox 命令分类](../agents/shellguard/index.md) — 只读模式写效应拦截。
- [测试规则](../rules/) — 功能测试、回归 gate、测试分层。

## 变更记录

- [Changelog](changelog/) — 按日期记录变化与验证。
