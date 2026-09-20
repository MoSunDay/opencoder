Commit: 7e71cbcfd669dd2cbaa5c94ab01945fd139557f0

# OpenCoder 能力地图 — 业务能力总索引

## 平台与编排

- [Agent 调度平台](agent-platform/index.md) — Server/Node 调度与按 ID 查执行明细。
- [DAG 工作流](../agents/dag-runtime/index.md) — agent/wasm 步骤、[dynamic 模板与实例批次](../docs/dag-dynamic.md)、依赖执行与产物、单步执行记录（`session.json` / `step_output`）；控制台点画布任一 step 打开单步执行记录抽屉（wasm=实时日志、agent=类 TUI 会话转写），走 SSE `/api/dag/runs/:id/steps/:step/events`。定义编辑器画布支持工具栏「连线」模式两段式点击建依赖边（armed 源高亮、合法目标提示，`canConnect` 拒自连/重复/成环，Esc/点空白取消），点 Handle 未拖动同样进入待目标模式，Handle 拖拽与 JSON 模式仍可用；运行画布保持只读、点节点开抽屉。
- [持久化 TODO 工作流](todos/index.md) — 父会话调度验收、独立 TODO 执行。
- 发布门禁（code-review DAG）— 内置 9 步知识库变更审查工作流
  （`examples/dag/code-review.json`：kb-index wasm 索引 → 范围 → API/护网双锚定 →
  双路审查 → 重核 → 裁决 → viking 工单）+ `scripts/platform/code_review_gate.sh`
  CLI 门禁（verdict=pass 才 exit 0）；入口为 compat dispatch 的
  `input.prompt=base=… head=…`。
- [项目管理](../agents/project/index.md) — goal/milestone/todo 与多执行器。
- [版本化 Agent 与 NFS](../agents/agents/index.md) — 按 Agent 编辑四类资源、专属版本与共享池、只读导出；无全局激活（默认 agent 走 `--agent` > `config.agent.default` > "act"），memory 支持目录化多文件（`.md` 字典序聚合注入），资源树行内新建/改名；引用卡 `run_mode: agent` 的 `kind=agent` 会话在节点上每回合跑 runc 容器（fail-closed 准入，见 [worker](../agents/worker/index.md)）。
- [大脑调度工作台](brain/index.md) — 版本化本体计划、并发执行画布与回执恢复。
- [大脑能力库](../agents/brain/index.md) — 能力录入、语义检索与路由。
- [远程管理 CLI](../agents/ctl/index.md) — 对接 Server API 与退出码契约。

## 会话与交互

- [会话运行时](../agents/session/index.md) — act/plan、压缩、subagent、恢复。
- [Agent Harness](harness/index.md) — opencode/codex 执行器与资源快照。
- [CLI](../agents/local/index.md) — 无头运行、TODO 命令与工具安装。
- [TUI](../agents/tui/index.md) — Turn 阶梯、模式切换与复制；`/agent` 命令模糊选择 primary agent。
- [Web 会话](../agents/web/index.md) — 流式会话、SSE 与模型发现；节点对话侧栏支持悬停删除与底部一键清空（终态即删、运行中保留）；输入框 `@`/`/agent` 模糊提及 agent（有会话即切、无会话入创建参数）。

## 配置与基础能力

- [配置和资源作用域](../agents/core/index.md) — 模型/压缩/命名环境与 Skill 注入。
- **内置 Skill 工作流**：`task-plan` 输出从现状到上线的闭环计划，`task-plan-subagent` 将计划拆成有 owner、写集、依赖和验收证据的 subagent 任务，`say-and-replay` 在任意检查点以 REPLAY 块回放目标、进度、证据、卡点和剩余 TODO；三者均随二进制增量 seed 到全局 skill 目录。
- [模型客户端](../agents/llm/index.md) — OpenAI 兼容流式与测试夹具。
- [持久化](../agents/store/index.md) — 本地存储与会话/执行状态。
- [sandbox 命令分类](../agents/shellguard/index.md) — 只读模式写效应拦截。
- [测试规则](../rules/) — 功能测试、回归 gate、测试分层。

## 变更记录

- [Changelog](changelog/) — 按日期记录变化与验证。
