Commit: a7e6b8233d5a4f6d0727d656d5d3d45a985b4b63

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- `src/lib.rs` — `AppState` 装配（`config_home`：Operator 执行 home，prompt/config 载入走 `Config::load_with_home`，drain 栈经 `DrainContext` 穿参）
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- [api_transcript.rs](../../crates/web/src/api_transcript.rs) — `GET /api/sessions/:id/transcript` 按消息序号和字节偏移读取完整展示消息块；TUI 经 Server 执行命令的 `http` 转发调用，复用已有渲染。
- `src/api_agents.rs`、`src/api_agent_resources.rs` — agent 目录/卡片与资源文件 API
- [api_dag_binaries.rs](../../crates/web/src/api_dag_binaries.rs)、[api_dag_binaries_nfs.rs](../../crates/web/src/api_dag_binaries_nfs.rs)、[api_dag_workspace_nfs.rs](../../crates/web/src/api_dag_workspace_nfs.rs) — 二进制版本池与独立只读导出管理；控制面和资源服务共享处理器，上传请求使用统一体积上限。
- `src/handle.rs`、`src/handle/drain.rs` — `SessionHandle` 与 drain 生命周期
- `src/auth_mw.rs`、`src/html.rs` — Bearer → Identity，独立指标凭据仅接受 `GET /metrics`；SPA 产物内嵌与 `/static` 白名单
- `src/api_control.rs` — 节点对话 API
- `spa/src/` — React18+antd SPA（vitest）；`src/html.rs` 在编译时嵌入已提交的 `spa/dist`，修改页面后须重建产物
- [SPA 产物检查](../../scripts/check-spa-drift.sh) — 在临时目录用 SPA 源码与包内资源重建，逐文件比较 `dist`；不复制仓库示例目录，不重试掩盖差异。
- `spa/src/chat.jsx`、`spa/src/chatSidebar.jsx`、`spa/src/chat/` — 会话页（Operator/Agent 双模式 lane）；Operator 创建前可选 Codex Harness 与逐行 env，随 `/api/sessions` 创建请求发送，启动后固定。`app.css` 在窄屏将会话侧栏与输入区纵向排列，保持输入区可操作
- `spa/src/main.jsx` — 身份确认完成后才挂载导航与页面；身份格式错误和读取失败提供重试，401 返回登录入口。
- [nav.js](../../crates/web/spa/src/nav.js)、[shell/categoryTabs.jsx](../../crates/web/spa/src/shell/categoryTabs.jsx) — 项目、Agent、Ontology、节点四类导航；标签保持完整宽度，容器支持滚轮、触摸和键盘滚动，并保持当前标签可见。非管理员可打开全部执行和全部 Ontology 页面。
- [ontology/panels.tsx](../../crates/web/spa/src/ontology/panels.tsx)、[ontology/env.tsx](../../crates/web/spa/src/ontology/env.tsx) — TypeScript + antd 的五个 Ontology 页面，复用平台身份与请求；图谱使用 G6，环境切换重新建立页面状态，管理控件由服务端能力决定。接口与存储见 [ontology](../ontology/index.md)。
- `spa/src/ui/requests/query.js` — 读取请求的取消、迟到响应丢弃、响应校验与错误状态；失败不替换为空数据。
- `spa/src/fleet/`、`spa/src/schedule/` — 执行表与定时任务页；`schedule/history.jsx` 按历史记录的执行 ID 打开原执行，不重新派发。节点调度设置读取失败时禁止保存默认值。
- [fleet/detail.jsx](../../crates/web/spa/src/fleet/detail.jsx)、[fleet/detail/workloads.jsx](../../crates/web/spa/src/fleet/detail/workloads.jsx) — TODO 执行明细在工作流建立后加载工作台；初始化、停止和初始化失败只展示对应状态，初始化错误只显示一次。工作流建立后的读取错误仍显示实际原因。
- `src/api_project*.rs` — 项目、专项、TODO 与 Tag 的共享 HTTP 处理器；Tag 范围和选择经存储验证，顺序写入携带 `initiative_id` 范围，负数位置保留给迁移且 API 拒绝；Control 复用同一组处理器
- `spa/src/project/`、`views/projectTable.jsx`、`views/viewState.jsx` — 三个表格与列筛选，视图状态在保存刷新及抽屉关闭后保留；项目和专项分别进入 `views/projectDrawer.jsx`、`views/initiativeDrawer.jsx`
- `spa/src/project/board/`、`model/board.js`、`model/catalog.js` — dnd-kit 看板与纯移动、进度、Tag 解析；按完整任务集合计算筛选后的拖动顺序，多 Tag 卡片共享 TODO ID，失败回退原数据
- `spa/src/project/todoDrawer.jsx`、`execute/launcher.jsx` — TODO 编辑、Tag 选择、指派历史与原生执行界面；复用 `ExecutionView` 的记录和引导入口，窄屏表格内部横向滚动
- `spa/src/dag/editor/canvasEditor.jsx` — 用已发出的 spec 签名避免重复通知，并保证依赖边更新在节点编辑之后仍能保存
- `spa/src/agents/`、`spa/src/agentNfsCard.jsx` — Agent 配置与资源页签；复用状态卡读取 Agent、二进制、源工作区、Ontology 正文四个实际只读 NFS 导出，停止须确认，读取失败不显示为已停止。
- `spa/src/dag/resources/` — 二进制池界面；`model.js` 负责 ELF 与版本引用纯校验，`read.js` 校验池和历史响应，`editor.jsx` 发布文件，`panel.jsx` 管理下载、删除与当前版本指针，`field.jsx` 为步骤选择受理时 current 或固定版本。
- `spa/src/dag/` — DAG 定义/运行页签与 React Flow 图：运行图 `dagProjection.js#graphFromSpec`（纯投影）与编辑器 `editor/canvasModel.js#specToCanvas` 的节点均声明固定盒（width + handles，**不声明 height**——声明会把内联高度烤进 wrapper，钳死 auto-height 卡片并错位 handle/fitView），边 id 用 `'e-' + src + '>' + dst`（`>` 不在 slug 字符集，杜绝连字符撞 key；`onConnect` 的 addEdge 路径同样显式传 id，勿依赖默认 getEdgeId）；RF 边是「两端节点 initialized（只需宽度）才渲染」的门控，勿再移除声明盒（jsdom RO shim 不回调，DOM 测试 `.react-flow__edge` 断言依赖声明盒）；编辑器 fitView 走 `useNodesInitialized()` 门控 + once-guard（仅挂载后首帧 fit，加步骤引起的重测量不再 refit）+ autoLayout `fitEpoch` effect，勿回退定时器
- `spa/src/dag/` spec 顶层 `max_concurrency` — 整跑并发上限（1..=30，缺省省略键、走服务端默认 4）：画布基础信息面板 `editor/stepInspector.jsx#SpecMetaForm` 可编辑；`editor/canvasModel.js#canvasToSpec` 透传 baseSpec 值（画布结构编辑不丢并发配置）；`specValidate.js` 镜像常量 `MAX_CONCURRENCY = 30` 在 `validateSpec` 校验（先于 name 检查）；只改定义、不影响在跑 run（run 持有 `dag_runs.spec_json` 快照，服务端整跑并发上限现状见 `crates/dag-runtime/src/runtime/scheduler.rs#schedule`）
- `spa/src/dag/dynamic/`、`spa/src/brain/workbench/` — 动态 DAG 与 Brain 工作台
- [dag/step/binaryLogs.jsx](../../crates/web/spa/src/dag/step/binaryLogs.jsx) — 原生二进制步骤输出；编辑器只提供 Binary、Agent 与 Dynamic，运行页面按节点所属执行读取实例、日志与声明产物。
- [dag/run/context.jsx](../../crates/web/spa/src/dag/run/context.jsx) — DAG 与执行明细共用的只读运行环境；容器、工作目录、版本和摘要来自节点保存的运行资料，不从当前资源池推测历史。
- `spa/src/brain/workbench/` — schema 7 工作台。`scheduler/editor.jsx` 在 `milestone/` 画布上配置必填里程碑信息和绑定泛化能力的并行节点，随后用表单提交计划信息；节点名称和任务由能力库生成。画布按顺序展示相邻层，大脑在运行时决定是否回到已执行层。`milestone/run.jsx` 保持状态与画布为主视图，`runDetails.jsx` 将历史激活按轮次汇入右侧抽屉表格，执行记录复用 `ExecutionView`；右侧抽屉中的人工输入，以及受计划管理的 Agent/Operator/Team 明细引导，统一提交到 Brain 输入事件；托管明细不暴露单独中断/取消执行的控件。`useRun.js` 读取 `/layered`，由事件流及轮询刷新。
- `tests/` — 集成测试

## 接缝
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。
- [全站验收入口](../../scripts/acceptance/ui/main.js) 以 `nav.js` 注册页和 `ui/scope.js` 功能覆盖表为范围，校验成套构建、SPA 产物、四种屏宽、真实功能与 Server TUI；缺页、异常、超时或构建摘要不一致使验收失败。
- 验收 `--resume` 只复用覆盖范围及二进制摘要相同的成功记录，失败保留日志并重跑，SPA 漂移检查始终执行；`--brain-test` 可使用同批构建的测试可执行文件，其原生镜像版本检查仍然执行。

## 相关
- [control](../control/index.md)、[brain](../brain/index.md) — Brain 校验/快照发布/执行在后端
- [动态 DAG 步骤](../../docs/dag-dynamic.md)
- [DAG 能力](../../features/dag/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
- [UI 验收约定](../../rules/05-ui-acceptance.md)
