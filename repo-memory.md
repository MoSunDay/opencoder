Commit: 4a6a4181c3cce8359e1668735b4e69695a0daf2a

# OpenCoder 逻辑地图

Rust 原生编码代理 workspace：`opencoder`（本地 CLI/TUI）、`opencoder-cli`（远程管理）、`opencoder-server`（版本与只读资源服务）、`opencoder-agent`（节点入口）。另有测试用途二进制 `mcp_mock_server`（crates/session）。

仓库只保留通用基建实现和说明；具体任务及其上下文通过运行时接入，遵循 [仓库范围约定](rules/08-infrastructure-scope.md)。

抽象口子：`Arc<dyn Store>`、`Arc<dyn ChatStream>`。细节见各模块索引，代码是最终事实。

## 模块索引

**foundation**

- [agents/core](agents/foundation/core/index.md) — 共享类型与 Config。
- [agents/llm](agents/foundation/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` + `MockChatClient` + token 估算。
- [agents/store](agents/foundation/store/index.md) — `Store` trait 与 Turso（WAL）主后端；Store、Fleet、TsRegistry、Ontology 共用 crates/database 的内嵌 turso 引擎，全部写路径经 `database::busy` 的 BEGIN IMMEDIATE 有界 Busy 重试（25ms→1s，预算 120s）取跨进程写锁；schema 就绪的暖打开经只读版本探针跳过写锁引导（不等待跨进程写锁）。
- [agents/event](agents/foundation/event/index.md) — `opencoder-event`：事件日志、监听器、广播总线、触发执行与全局暂停门；纯内存、零存储引擎依赖；通用基建，未接生产 dispatcher。

**runtime**

- [agents/session](agents/runtime/session/index.md) — 会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。
- [agents/shellguard](agents/runtime/shellguard/index.md) — shell 安全分类器：rable AST 判定、释放集仅 `/tmp`+`/dev/null`、fail-closed。

**frontend**

- [agents/local](agents/frontend/local/index.md) — 本地 CLI 前端：参数解析、headless、tmux 会话入口。
- [agents/tui](agents/frontend/tui/index.md) — ratatui 交互界面（Step 阶梯默认折叠、计数样式不变（运行中折叠组行 + running 提示）+ turn 实时区经 flatten 内联在运行中阶梯下方、Subagent 块之上（阶梯只渲染已落定 step，首个 in-flight step 期间阶梯零行），`> streaming` 头行默认展开、点击/Ctrl+L 可收起，与转录共用同一滚动窗，settle 后完整 step 落入阶梯：`chat_live_turn`（见 features/changelog/2026-11-09/live-region-inline-below-ladder-collapsible.md）；open Say 一开始其阶梯整体退场（与 live 区同时清空，流式期间转录仅 `say:` 头行 + running 提示 + raw body 全量，共享滚动），seal 后合并为 `Say(n steps)` 头行、步数回归（见 features/changelog/2026-11-09/tui-ladder-retires-on-say-stream.md）；一次性 `@capability` 子任务派发）。
- [agents/web](agents/frontend/web/index.md) — axum HTTP + SSE 会话管理 + 内嵌 SPA（Step 阶梯默认折叠、计数样式不变 + turn 实时区 `TurnLiveRegion` 位于 transcript 滚动容器内、消息列表之下、与消息共用滚动条：头部 `> streaming` 展开 / `▸ streaming` 收起（dim warn、默认展开、点击头行或 Ctrl/Cmd+L 收起、每个新 turn 重新展开，折叠仅隐藏主体以保留嵌套调用开合状态），主体仅 thinking + 与阶梯 L3 同款调用行（无 Step(k) 计数、无 Say 块）；流式 Say 由 `StreamingSay`（`pendingSay` 原文）渲染在同一滚动流、flush 边界落地为消息，auto-stick 仅贴底跟随；settle 后实时区清空（`findRunningStepsTurn`）：`stepsBlock.jsx`）。

**ontology**

- [agents/ontology](agents/ontology/index.md) — 独立 `ontology.db`（内嵌 turso 引擎）的类型、实体、关系、图谱切面、目录多归属、数据保鲜与不可变正文。

**workflow**

- [agents/dag](agents/workflow/dag/index.md) — DAG 纯域 + 线协议（DTO LOCKED）。
- [agents/dag-binary](agents/workflow/dag-binary/index.md) — Linux 二进制版本池、只读资源分发与版本固定。
- [agents/dag-runtime](agents/workflow/dag-runtime/index.md) — 每次 DAG 运行归属单个节点、单个容器，共享写时复制工作区；Server 不链接。

**orchestration**

- [agents/brain](agents/orchestration/brain/index.md) — 能力库、分层调度与嵌套计划。
- [agents/team](agents/orchestration/team/index.md) — 团队目录与消息扇出运行时。

**workbench**

- [agents/project](agents/workbench/project/index.md) — 项目、专项、TODO 与 Tag 数据；旧执行链独立保留。
- [agents/todos](agents/workbench/todos/index.md) — 持久化 TODO 工作流：每 TODO 独立 Primary Session。

**platform**

- [agents/control](agents/platform/control/index.md) — 平台控制面：节点调度、五字段执行索引。
- [agents/server](agents/platform/server/index.md) — 版本 Server 与独立只读资源服务入口。
- [agents/node](agents/platform/node/index.md) — 出站 WebSocket：注册、心跳、RPC。
- [agents/agent](agents/platform/agent/index.md) — 稳定 Host、独立版本 Runtime 与兼容节点入口。
- [agents/agents](agents/platform/agents/index.md) — 版本化自定义 Agent：共享池 `v{n}` + meta.json 引用卡 + NFS 只读导出。
- [agents/ctl](agents/platform/ctl/index.md) — `opencoder-cli`：Server API + Bearer + 退出码约定；本机 Turso Storage 维护命令。

**execution**

- [agents/worker](agents/execution/worker/index.md) — 节点执行面：接受/恢复、资源快照、操作适配。
- [agents/computer-use](agents/execution/computer-use/index.md) — 可选 `opencoder-computer`：Cua 桌面任务 CLI、独立模型与运行产物。

OpenCoder 能力入口见 [features/index.md](features/index.md)。

## 仓库规则

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个功能必须有测试
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 改动及直接影响链路定向回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层（unit/integration/e2e）
- [rules/04-dag-execution-contract.md](rules/04-dag-execution-contract.md) — DAG 单节点单容器与原生步骤执行约定
- [rules/05-ui-acceptance.md](rules/05-ui-acceptance.md) — 只验收本次修改的页面与直接影响链路，页面范围由 `scripts/acceptance/ui/scope.js` 维护
- [rules/06-brain-scheduling-contract.md](rules/06-brain-scheduling-contract.md) — 大脑整层调度、结果评估、返工与恢复约定
- [rules/07-project-module-contract.md](rules/07-project-module-contract.md) — 项目、专项、TODO 的归属、看板、执行关联与进度约定
- [rules/08-infrastructure-scope.md](rules/08-infrastructure-scope.md) — 代码、样例、提示词与文档只保留通用基建内容
- [rules/09-ontology-capabilities.md](rules/09-ontology-capabilities.md) — Ontology 是全部平台能力的统一查询与组织入口，查找能力先走能力目录与图谱
- [rules/10-ontology-cli.md](rules/10-ontology-cli.md) — `opencoder-cli ontology` 的命令入口与操作方式，CLI/API 约定变化时同步更新
- [rules/11-test-organization.md](rules/11-test-organization.md) — 测试组织与模块地图：新增 case 先查地图归位、复用 support 夹具、验收脚本进 scripts/acceptance/<domain>/
