Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# OpenCoder 能力地图

## 平台与编排

- [Agent 调度平台](agent-platform/index.md)：独立 Server/Node、节点注册、CPU 调度、四字段索引、按 ID 读取节点明细、团队职责、系统维护团队、大脑能力绑定与直接派发。
- [DAG 工作流](../agents/dag-runtime/index.md)：agent/Python 步骤、依赖执行、本地产物、检查点恢复及 runc 支持。
- [持久化 TODO 工作流](todos/index.md)：父会话调度验收、独立 TODO 执行、依赖/并发、恢复、回退和工具验收合同。
- [项目管理](../agents/project/index.md)：goal/milestone/todo 结构、草稿、Plan → Act、执行记录和取消。
- [版本化 Agent 与 NFS 资源](../agents/agents/index.md)：引用卡、prompt/skills/tools/memory 共享池、版本发布/回滚和只读导出。
- [大脑能力库](../agents/brain/index.md)：能力录入、嵌入、语义检索及决策树规划。

## 会话与交互

- [会话运行时](../agents/session/index.md)：act/plan、显式交接、恢复与分叉、steer/queue、压缩、subagent、question、侧车问询及 autopilot。
- [CLI](../agents/cli/index.md)：无头运行、模型选择、会话导出/导入、TODO 命令和工具安装。
- [TUI](../agents/tui/index.md)：Turn 阶梯、推理显示、技能选择、模式切换、notepad、快捷键、文本复制（含 [Say 合并头预览载荷](changelog/2026-09-05/tui-copy-mode-say-pair-payload.md)）和上下文展示。
- [Web 会话](../agents/web/index.md)：流式会话、问题作答、排队/指导、模型/技能发现、annotation/autopilot、标题和 SSE 重连。

## 配置与基础能力

- [配置和资源作用域](../agents/core/index.md)：模型/压缩配置、按域文件、命名环境、CLI/MCP 注入范围和 Skill 上下文注入。
- [模型客户端](../agents/llm/index.md)：OpenAI 兼容流式协议、重试、超时、嵌入和确定性测试。
- [持久化](../agents/store/index.md)：本地高性能存储、会话与执行状态、恢复及独立平台索引。
- [sandbox 命令分类](../agents/shellguard/index.md)：只读模式下的 shell 写效应分类与拦截。
- [测试规则](../rules/)：功能测试、全量回归 gate、测试分层与执行证据。

## 变更记录

[Changelog](changelog/) 按日期记录可检索的变化和验证结果。
