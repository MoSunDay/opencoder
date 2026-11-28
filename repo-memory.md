Commit: 024175736

# OpenCoder 逻辑地图

Rust 原生编码代理 workspace：`opencoder`（本地 CLI/TUI）、`opencoder-cli`（远程管理）、`opencoder-server`（版本与只读资源服务）、`opencoder-agent`（节点入口）。另有测试用途二进制 `mcp_mock_server`（crates/session）。

仓库只保留通用基建实现和说明；具体任务及其上下文通过运行时接入，遵循 [仓库范围约定](rules/08-infrastructure-scope.md)。

抽象口子：`Arc<dyn Store>`、`Arc<dyn ChatStream>`。细节见各模块索引，代码是最终事实。

## 模块索引

**foundation**

- [agents/core](agents/foundation/core/index.md) — 共享类型与 Config；Shared/Act/Plan 模型/生成参数覆盖、本地 Env 文件分层与保存路由；进程级模型按 `--model` > `OPENCODER_MODEL` > Resume/Fork 会话选择 > per-mode config，显式覆盖保留生成参数与 context overlay。默认 `Config::load` 均读取 active Env，严格加载与 Env 路由保存单次解析 marker 并复用结果；显式 `Some(home)`/frozen/operator 加载隔离，非 `NotFound` I/O 错误 fail closed，stale 回退，legacy 路由不被活动 Env 劫持。
- [agents/llm](agents/foundation/llm/index.md) — OpenAI 兼容流式客户端 + `ChatStream` + `MockChatClient` + token 估算；pre-stream 重试 10 次、单次等待 ≤5 min。
- [agents/store](agents/foundation/store/index.md) — `Store` trait 与 Turso（WAL）主后端；Store、Fleet、TsRegistry、Ontology 共用 crates/database 的内嵌 turso 引擎（池打开即挂载周期 PASSIVE checkpoint：60s/1MiB 门限、busy 即跳过），全部写路径经 `database::busy` 的 BEGIN IMMEDIATE 有界 Busy 重试（25ms→1s，预算 120s）取跨进程写锁；schema 就绪的暖打开经只读版本探针跳过写锁引导（不等待跨进程写锁）；会话活跃索引 v41 改为普通 `(updated_at DESC, id DESC)`，`IdxDelete` 失步自动重建，坏行列表降级点查；schema v42 的可空 `sessions.model_overrides_json` 支持 Shared/Act/Plan，`{}`/缺省字段继承配置、Shared 值优先于 `modes.shared.model`、整体 `NULL` 保持 legacy；所有生产路径的新会话创建者均显式写 `Some(default())`，底层 `create_session` 与显式历史/legacy import 保留 `NULL`，历史行与旧 bundle 元数据不回填。
- [agents/event](agents/foundation/event/index.md) — `opencoder-event`：事件日志、监听器、广播总线、触发执行与全局暂停门；纯内存、零存储引擎依赖；通用基建，未接生产 dispatcher。

**runtime**

- [agents/session](agents/runtime/session/index.md) — 会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel；按 Shared/Act/Plan 解析模型、生成参数与上下文限制，完整会话选择随 resume/fork 及关联新任务保留，Shared 选择立即优先于 `modes.shared.model`，`take_model_switch` 按完整有效模型去重事件；显式模型对模式感知行写 `{}`、legacy 行保持 `NULL`，Env reload 持久化目标 Shared 并清空全部会话选择，Store 错误传播且补偿模型行/内存状态；懒行仅回滚确认本次新建的行，append 外键失败时按内存重建已删除会话行并重试。
- [agents/shellguard](agents/runtime/shellguard/index.md) — shell 安全分类器：rable AST 判定、释放集仅 `/tmp`+`/dev/null`、fail-closed。

**frontend**

- [agents/local](agents/frontend/local/index.md) — 本地 CLI 前端：参数解析、headless、tmux 会话入口；`--model` > `OPENCODER_MODEL` > Resume/Fork 的 Shared/Act/Plan 会话选择 > per-mode config，恢复后重应用同一显式胜者并保留生成参数/context overlay，新会话保持懒建，持久化失败传播且补偿磁盘与内存状态。
- [agents/tui](agents/frontend/tui/index.md) — ratatui 交互界面，支持 Shared/Act/Plan 模型菜单、本地 Env 管理及 Provider 新增/编辑/删除统一的磁盘/仅会话确认；Shared 会话选择立即优先于 `modes.shared.model`，`ApplySession`/`ReloadEnv` 仅在成功 ack 后提交外层 config/client/chat，失败保留原运行时；单独模型切换复用 client，Provider 变化才重建，`/task` New 继承完整 providers 并基于同一 `new_config` 构造 client，Resume/Fork 重应用显式模型且仅在新 worker 就绪后提交切换；普通 ReloadConfig 与 `/cli` `/skill` 失败路径不提交未成功的 live 状态（Step 阶梯默认折叠、计数样式不变（运行中折叠组行 + running 提示）+ turn 实时区经 flatten 内联在运行中阶梯下方、Subagent 块之上（阶梯只渲染已落定 step，首个 in-flight step 期间阶梯零行），`> streaming` 头行默认展开、点击/Ctrl+L 可收起，与转录共用同一滚动窗，settle 后完整 step 落入阶梯：`chat_live_turn`（见 features/changelog/2026-11-09/live-region-inline-below-ladder-collapsible.md）；open Say 一开始其阶梯整体退场（与 live 区同时清空，流式期间转录仅 `say:` 头行 + running 提示 + raw body 全量，共享滚动），seal 后合并为 `Say(n steps)` 头行、步数回归（见 features/changelog/2026-11-09/tui-ladder-retires-on-say-stream.md）；夹在阶梯与其 Say 之间的 Subagent 块对配对透明（`chat_steps.rs::say_after_group`/`group_before_say`，flatten/headers 共用同一查找，见 features/changelog/2026-11-23/subagent-transparent-step-say-pairing.md）；该段整体提到 Say 对上方渲染、段与 Say 之间空一行（`chat_steps.rs::render_order`/`opens_with_say`，flatten/headers/测试镜像共用，见 features/changelog/2026-11-27/subagent-section-above-say.md）；每段极大连续 Subagent 块前置一行 `> subagents` / `▸ subagents` 分组头（dim warn、与 live 区头行同款；视图级 `ChatView::subagents_collapsed` 跨 turn 持久，点击组头一次折叠 / 展开该视图全部分组、Ctrl+L 只收起，收起态 subagent 行不渲染但组头保留，copy 保留组头；`chat_flatten.rs` / `chat_headers.rs::subagent_group_headers`，见 features/changelog/2026-11-27/subagent-fold-toggle.md）；一次性 `@capability` 子任务派发）；回合失败 Error 事件按 per-turn 标志去重。
- [agents/web](agents/frontend/web/index.md) — axum HTTP + SSE 会话管理 + 内嵌 SPA（Step 阶梯默认折叠、计数样式不变 + turn 实时区 `TurnLiveRegion` 位于 transcript 滚动容器内、消息列表之下、与消息共用滚动条：头部 `> streaming` 展开 / `▸ streaming` 收起（dim warn、默认展开、点击头行或 Ctrl/Cmd+L 收起、每个新 turn 重新展开，折叠仅隐藏主体以保留嵌套调用开合状态），主体仅 thinking + 与阶梯 L3 同款调用行（无 Step(k) 计数、无 Say 块）；流式 Say 由 `StreamingSay`（`pendingSay` 原文）渲染在同一滚动流、flush 边界落地为消息，auto-stick 仅贴底跟随；settle 后实时区清空（`findRunningStepsTurn`）：`stepsBlock.jsx`；run 内夹在阶梯与其 Say 之间的 subagent 块吸收进同一气泡（`bubbleItems.js::itemsFromTurns`，`transcript.jsx` 把该段渲染在头行上方（不切入 Say），见 features/changelog/2026-11-27/subagent-section-above-say.md））。

**ontology**

- [agents/ontology](agents/ontology/index.md) — 独立 `ontology.db`（内嵌 turso 引擎）的类型、实体、关系、图谱切面、目录多归属、数据保鲜与不可变正文；既有 `platform:tui` 登记六个 TUI 命令 `/env`、`/envs`、`/model`、`/config`、`/cli`、`/skill`，包含 Shared 会话选择的即时优先级、Provider session-only 选择、`/cli`/`/skill` 的 `EnvAware` 保存及 Resume/Fork 外层运行时同步；复用 `/api/tui` 查询，不新增公共 HTTP 路由或 `opencoder-cli` 命令域。

**workflow**

- [agents/dag](agents/workflow/dag/index.md) — DAG 纯域 + 线协议（DTO LOCKED）。
- [agents/dag-binary](agents/workflow/dag-binary/index.md) — Linux 二进制版本池、只读资源分发与版本固定。
- [agents/dag-runtime](agents/workflow/dag-runtime/index.md) — 每次 DAG 运行归属单个节点、单个容器，共享写时复制工作区；沙箱 `/` 是节点数据目录下运行目录内空锚点（`bundle/root`）上的只读 tmpfs，宿主系统目录与运行器目录（`dag.runners_dir`，回退 `<dag.rootfs_dir>/usr/bin`，绑定在 `/run/opencoder/bin`）只读绑定其上，不再复制 rootfs 镜像；Server 不链接。

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

- [agents/worker](agents/execution/worker/index.md) — 节点执行面：接受/恢复、资源快照、操作适配；默认 `Config::load` 均读取 active Env，显式 `Some(home)`/frozen/operator 加载保持隔离；配置缓存指纹覆盖活动 Env、发现目录身份及无活动 Env 时纯 XDG 三类文件的创建/同长编辑/删除，并区分 `Missing`/`Regular`/`Directory`/`Other`，元数据错误不得复用旧缓存；Operator 以一次严格 Config+raw-domain 同源快照初始化，私有按目录 `.bootstrap-lock` 串行化并在锁后复查 marker，完整 bootstrap 后才发布 completion marker，marker 存在即冻结、缺失时失败可重试；持久化闩锁指数退避自愈（成功派发周期即清双闩锁，读取统一走 node_error/error 访问器）。
- [agents/computer-use](agents/execution/computer-use/index.md) — 可选 `opencoder-computer`：Cua 桌面任务 CLI、独立模型与运行产物。

OpenCoder 能力入口见 [features/index.md](features/index.md)。

## 仓库规则

- [rules/01-mandatory-tests.md](rules/01-mandatory-tests.md) — 每个功能必须有测试
- [rules/02-regression-gate.md](rules/02-regression-gate.md) — 改动及直接影响链路定向回归 + changelog 附测试清单
- [rules/03-test-pyramid.md](rules/03-test-pyramid.md) — 测试分层与组织：unit/integration/e2e 判层、目录落位、夹具复用与验收脚本归组
- [rules/04-dag-execution-contract.md](rules/04-dag-execution-contract.md) — DAG 单节点单容器与原生步骤执行约定
- [rules/05-ui-acceptance.md](rules/05-ui-acceptance.md) — 只验收本次修改的页面与直接影响链路，页面范围由 `scripts/acceptance/ui/scope.js` 维护
- [rules/06-brain-scheduling-contract.md](rules/06-brain-scheduling-contract.md) — 大脑整层调度、结果评估、返工与恢复约定
- [rules/07-project-module-contract.md](rules/07-project-module-contract.md) — 项目、专项、TODO 的归属、看板、执行关联与进度约定
- [rules/08-infrastructure-scope.md](rules/08-infrastructure-scope.md) — 代码、样例、提示词与文档只保留通用基建内容
- [rules/09-ontology-capabilities.md](rules/09-ontology-capabilities.md) — Ontology 能力目录与 CLI：统一查询入口、同步要求与 opencoder-cli ontology 操作约定
