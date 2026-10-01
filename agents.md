Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# OpenCoder 逻辑地图

Rust 原生编码代理 workspace：`opencoder`（本地 CLI/TUI）、`opencoder-cli`（远程管理）、`opencoder-server`（控制面）、`opencoder-agent`（节点执行）。

抽象口子：`Arc<dyn Store>`、`Arc<dyn ChatStream>`。细节见各模块索引，代码是最终事实。

## 仓库记忆范围

`agents.md`、`agents/` 和 `features/`（含 changelog）只记录 OpenCoder 本身的模块、通用基建能力、接口契约及其演进。外部业务系统的需求、业务逻辑、数据结论、部署现场、巡检结果和一次性执行回执不得写入本仓库记忆。涉及外部系统的工作，仅在解释 OpenCoder 自身接口或能力边界所必需时记录通用事实，不沉淀具体业务状态。

## 模块索引

- [agents/core](agents/core/index.md) — 共享类型与 Config。
- [agents/llm](agents/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` + `MockChatClient` + token 估算。
- [agents/store](agents/store/index.md) — `Store` trait + libsql（WAL）持久化层。
- [agents/session](agents/session/index.md) — 会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
- [agents/shellguard](agents/shellguard/index.md) — shell 安全分类器：rable AST 判定、释放集仅 `/tmp`+`/dev/null`、fail-closed。
- [agents/tui](agents/tui/index.md) — ratatui 交互界面。
- [agents/local](agents/local/index.md) — 本地 CLI 前端：参数解析、headless、tmux 会话入口。
- [agents/web](agents/web/index.md) — axum HTTP + SSE 会话管理 + 内嵌 SPA。
- [agents/dag](agents/dag/index.md) — DAG 纯域 + 线协议（DTO LOCKED）。
- [agents/dag-binary](agents/dag-binary/index.md) — Linux 二进制版本池、只读资源分发与版本固定。
- [agents/dag-runtime](agents/dag-runtime/index.md) — 单 DAG 单容器、共享写时复制工作区与原生步骤调度；Server 不链接。
- [agents/todos](agents/todos/index.md) — 持久化 TODO 工作流：每 TODO 独立 Primary Session。
- [agents/project](agents/project/index.md) — 项目、专项、TODO 与 Tag 数据；旧执行链独立保留。
- [agents/brain](agents/brain/index.md) — 能力库、版本化 step/连线计划、唯一 v4 分层调度与嵌套计划。
- [agents/agents](agents/agents/index.md) — 版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。
- [agents/team](agents/team/index.md) — 团队目录与消息扇出运行时。
- [agents/control](agents/control/index.md) — 平台控制面：节点调度、五字段执行索引。
- [agents/worker](agents/worker/index.md) — 节点执行面：接受/恢复、资源快照、操作适配。
- [agents/node](agents/node/index.md) — 出站 WebSocket：注册、心跳、RPC。
- [agents/server](agents/server/index.md) — 版本 Server 与独立只读资源服务入口。
- [agents/agent](agents/agent/index.md) — 稳定 Host、独立版本 Runtime 与兼容节点入口。
- [agents/ctl](agents/ctl/index.md) — `opencoder-cli`：Server API + Bearer + 退出码约定。

OpenCoder 能力入口见 [features/index.md](features/index.md)。

## 仓库规则

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个业务功能必须有测试
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 迭代结束全量回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层（unit/integration/e2e）
- [rules/04-dag-execution-contract.md](rules/04-dag-execution-contract.md) — DAG 单节点单容器与原生步骤执行约定
