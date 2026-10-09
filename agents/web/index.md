Commit: df06a3b0a177c14b53171818717eaca0e0c35e64

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- [admin/users.jsx](../../crates/web/spa/src/admin/users.jsx)、[admin/tokens.jsx](../../crates/web/spa/src/admin/tokens.jsx) — 用户角色编辑与独立 Token 的签发、到期和撤销；新 Token 只展示一次。
- [chat/conversationInput.jsx](../../crates/web/spa/src/chat/conversationInput.jsx) — 会话页和 Agent/Operator 执行详情共用输入组件；按当前角色控制提交，保留引导、排队与问题交互。
- [chat/stream/completion.js](../../crates/web/spa/src/chat/stream/completion.js) — 流结束后对齐持久化消息及执行状态，恢复发送状态；迟到的旧请求不能覆盖新会话。
- `src/lib.rs` — `AppState` 装配（`config_home`：Operator 执行 home，prompt/config 载入走 `Config::load_with_home`，drain 栈经 `DrainContext` 穿参）
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- [api_transcript.rs](../../crates/web/src/api_transcript.rs) — `GET /api/sessions/:id/transcript` 按消息序号和字节偏移读取完整展示消息块；TUI 经 Server 执行命令的 `http` 转发调用，复用已有渲染。
- `src/api_agents.rs`、`src/api_agent_resources.rs` — agent 目录/卡片与资源文件 API
- [api_dag_binaries.rs](../../crates/web/src/api_dag_binaries.rs)、[api_dag_binaries_nfs.rs](../../crates/web/src/api_dag_binaries_nfs.rs)、[api_dag_workspace_nfs.rs](../../crates/web/src/api_dag_workspace_nfs.rs) — 二进制版本池与独立只读导出管理；控制面和资源服务共享处理器，上传请求使用统一体积上限。
- `src/handle.rs`、`src/handle/drain.rs` — `SessionHandle` 与 drain 生命周期
- `src/auth_mw.rs`、`src/html.rs` — Bearer → Identity，独立指标凭据仅接受 `GET /metrics`；SPA 产物内嵌与 `/static` 白名单
- `src/api_control.rs` — 节点对话 API
- `spa/src/` — React18+antd SPA（vitest）；`src/html.rs` 在编译时嵌入已提交的 `spa/dist`，修改页面后须重建产物
- [harness/management.jsx](../../crates/web/spa/src/harness/management.jsx)、[configuration.js](../../crates/web/spa/src/harness/configuration.js) — Codex 默认配置与命名档案编辑模型和外部启动脚本；每行对应一个命令参数，空白表单保存为空数组。
- [SPA 产物检查](../../scripts/check-spa-drift.sh) — 在临时目录用 SPA 源码与包内资源重建，逐文件比较 `dist`；不复制仓库示例目录，不重试掩盖差异。
- `spa/src/chat.jsx`、`spa/src/chatSidebar.jsx`、`spa/src/chat/` — 会话页（Operator/Agent 双模式 lane）；Operator 创建前可选 Codex Harness 与逐行 env，随 `/api/sessions` 创建请求发送，启动后固定。`app.css` 在窄屏将会话侧栏与输入区纵向排列，保持输入区可操作；[modelModal.jsx](../../crates/web/spa/src/modelModal.jsx) 负责会话模型选择与切换。
- [chat/useTranscriptStream.js](../../crates/web/spa/src/chat/useTranscriptStream.js)、[chat/stream/completion.js](../../crates/web/spa/src/chat/stream/completion.js) — `sse.js` 的 `onEnd` 单独通知 `stream_end`；会话页限时读取执行结果与会话快照，恢复内容及结束状态，取消订阅使未完成的读取失效。
- `spa/src/main.jsx` — 身份确认完成后才挂载导航与页面；身份格式错误和读取失败提供重试，401 返回登录入口。
- [nav.js](../../crates/web/spa/src/nav.js)、[shell/categoryTabs.jsx](../../crates/web/spa/src/shell/categoryTabs.jsx) — 项目、Agent、Ontology、后台管理四类导航；标签保持完整宽度，容器支持滚轮、触摸和键盘滚动，并保持当前标签可见。editor 与 viewer 可打开平台页面，后台管理仅管理员可见；viewer 的写入控件禁用。
- [Ontology 前端](ontology.md) — 五个页面、图谱与结果列表、实体详情草稿及环境观测记忆；复用平台身份、请求与 antd。接口与存储见 [ontology](../ontology/index.md)。
- `spa/src/ui/requests/query.js` — 读取请求的取消、迟到响应丢弃、响应校验与错误状态；失败不替换为空数据。
- `spa/src/fleet/`、`spa/src/schedule/` — 执行表与定时任务页；`schedule/history.jsx` 按历史记录的执行 ID 打开原执行，不重新派发。节点调度设置读取失败时禁止保存默认值。
- [fleet/detail.jsx](../../crates/web/spa/src/fleet/detail.jsx)、[fleet/detail/workloads.jsx](../../crates/web/spa/src/fleet/detail/workloads.jsx) — TODO 执行明细在工作流建立后加载工作台；初始化、停止和初始化失败只展示对应状态，初始化错误只显示一次。工作流建立后的读取错误仍显示实际原因。
- [fleet/teams/](../../crates/web/spa/src/fleet/teams/) — Team 列表、成员编辑、启动表单与可复用的 `TeamExecutionHistory`；历史记录按服务端游标分页并自动刷新，点击打开原执行。`submission.js` 限制提交与回执读取的等待时间并校验执行身份，`useSubmission.js` 在提交超时或回复不确定后查询现有 `/receipt`，确认受理后跳转详情。重试保留原请求，关闭时取消读取并忽略迟到响应。
- [fleet/detail/team/](../../crates/web/spa/src/fleet/detail/team/) — 通用执行详情复用 `TeamExecutionProcess`，展示轮次、发言与小结；`progress.js` 读取当前未完成轮次已落盘的计划与小结，`record.jsx` 更新尚未生成的发言。读取有界、请求不重叠，澄清对象来自上次小结；切换执行取消旧请求，大记录保留分段读取，异常结束保留已保存的未完成轮次。
- `src/api_project*.rs` — 项目、专项、TODO 与 Tag 的共享 HTTP 处理器；Tag 范围和选择经存储验证，顺序写入携带 `initiative_id` 范围，负数位置保留给迁移且 API 拒绝；Control 复用同一组处理器
- `spa/src/project/`、`views/projectTable.jsx`、`views/viewState.jsx` — 项目、专项、TODO、Tag 四个表格与列筛选，视图状态在保存刷新及抽屉关闭后保留；项目和专项分别进入 `views/projectDrawer.jsx`、`views/initiativeDrawer.jsx`；专项详情与 `todoDrawer.jsx` 的外层 Drawer 均使用 `size="100vw"`，不设置固定最大宽度。
- [tags/tab.jsx](../../crates/web/spa/src/project/tags/tab.jsx)、[tags/editor.jsx](../../crates/web/spa/src/project/tags/editor.jsx)、[tags/model.js](../../crates/web/spa/src/project/tags/model.js) — 独立 Tag 管理表格、名称与归属编辑、定义行纯投影；保存失败保留草稿，切换归属类型清空旧归属。共享 [api_project_tags.rs](../../crates/web/src/api_project_tags.rs) 的创建和修改均校验 `name`、`scope_type`、`scope_id`，修改保留原 ID。
- `spa/src/project/board/`、`model/board.js`、`model/catalog.js` — dnd-kit 看板与纯移动、进度、Tag 解析；按完整任务集合计算筛选后的拖动顺序，多 Tag 卡片共享 TODO ID，失败回退原数据
- [project/todoDrawer.jsx](../../crates/web/spa/src/project/todoDrawer.jsx)、[project/execute/](../../crates/web/spa/src/project/execute/) — TODO 选择实际能力 ID，以稳定执行 ID 派发并关联；回复丢失时保留同一请求重试。结论在打开时向所属节点读取，失败清空旧结果并显示错误；`ExecutionView` 复用原会话、事件与引导入口。
- [chat/inputAttempt.js](../../crates/web/spa/src/chat/inputAttempt.js) — 会话页与执行明细共享人工输入 ID 的生成与重试规则，未确认的回复保持原输入 ID，避免断线重试重复提交。
- `spa/src/dag/editor/canvasEditor.jsx` — 用已发出的 spec 签名避免重复通知，并保证依赖边更新在节点编辑之后仍能保存
- `spa/src/agents/`、`spa/src/agentNfsCard.jsx` — Agent 配置与资源页签；复用状态卡读取 Agent、二进制、源工作区、Ontology 正文四个实际只读 NFS 导出，停止须确认，读取失败不显示为已停止。
- `spa/src/dag/resources/` — 二进制池界面；`model.js` 负责 ELF 与版本引用纯校验，`read.js` 校验池和历史响应，`editor.jsx` 发布文件，`panel.jsx` 管理下载、删除与当前版本指针，`field.jsx` 为步骤选择受理时 current 或固定版本。
- `spa/src/dag/` — DAG 定义/运行页签与 React Flow 图：运行图 `dagProjection.js#graphFromSpec`（纯投影）与编辑器 `editor/canvasModel.js#specToCanvas` 的节点均声明固定盒（width + handles，**不声明 height**——声明会把内联高度烤进 wrapper，钳死 auto-height 卡片并错位 handle/fitView），边 id 用 `'e-' + src + '>' + dst`（`>` 不在 slug 字符集，杜绝连字符撞 key；`onConnect` 的 addEdge 路径同样显式传 id，勿依赖默认 getEdgeId）；RF 边是「两端节点 initialized（只需宽度）才渲染」的门控，勿再移除声明盒（jsdom RO shim 不回调，DOM 测试 `.react-flow__edge` 断言依赖声明盒）；编辑器 fitView 走 `useNodesInitialized()` 门控 + once-guard（仅挂载后首帧 fit，加步骤引起的重测量不再 refit）+ autoLayout `fitEpoch` effect，勿回退定时器
- `spa/src/dag/` spec 顶层 `max_concurrency` — 整跑并发上限（1..=30，缺省省略键、走服务端默认 4）：画布基础信息面板 `editor/stepInspector.jsx#SpecMetaForm` 可编辑；`editor/canvasModel.js#canvasToSpec` 透传 baseSpec 值（画布结构编辑不丢并发配置）；`specValidate.js` 镜像常量 `MAX_CONCURRENCY = 30` 在 `validateSpec` 校验（先于 name 检查）；只改定义、不影响在跑 run（run 持有 `dag_runs.spec_json` 快照，服务端整跑并发上限现状见 `crates/dag-runtime/src/runtime/scheduler.rs#schedule`）
- `spa/src/dag/dynamic/`、`spa/src/brain/workbench/` — 动态 DAG 与 Brain 工作台
- [dag/step/binaryLogs.jsx](../../crates/web/spa/src/dag/step/binaryLogs.jsx) — 原生二进制步骤输出；编辑器只提供 Binary、Agent 与 Dynamic，运行页面按节点所属执行读取实例、日志与声明产物。
- [dag/run/context.jsx](../../crates/web/spa/src/dag/run/context.jsx) — DAG 与执行明细共用的只读运行环境；容器、工作目录、版本和摘要来自节点保存的运行资料，不从当前资源池推测历史。
- `spa/src/brain/workbench/` — schema 7 工作台。`scheduler/editor.jsx` 在 `milestone/` 画布上配置必填里程碑信息和绑定泛化能力的并行节点，随后用表单提交计划信息；节点名称和任务由能力库生成。画布按顺序展示相邻层，大脑在运行时决定是否回到已执行层。`milestone/run.jsx` 保持状态与画布为主视图，`runDetails.jsx` 将历史激活按轮次汇入右侧抽屉表格，执行记录复用 `ExecutionView`；右侧抽屉中的人工输入，以及受计划管理的 Agent/Operator/Team 明细引导，统一提交到 Brain 输入事件；托管明细不暴露单独中断/取消执行的控件。`useRun.js` 读取 `/layered`，由事件流及轮询刷新。
- [brain/workbench/plans.jsx](../../crates/web/spa/src/brain/workbench/plans.jsx) — 新建计划、点击当前计划名称与修改操作共用 `PlanEditor`；历史版本预览独立打开，旧格式必须显式转换后编辑。读取请求用序号隔离，迟到响应不能覆盖新建表单。
- [brainPanel.jsx](../../crates/web/spa/src/brainPanel.jsx)、[brain/library.css](../../crates/web/spa/src/brain/library.css) — 能力表格固定布局，一句话描述限制宽度并省略；输入、输出与工程示例由 `CapabilityEditor` 读取完整详情后展示，列表和搜索结果共用入口。
- `tests/` — 集成测试

## 接缝
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。
- [serve](../../crates/web/src/lib.rs) 为会话与 `ProjectService` 提供同一 `LibsqlStore` 实例，共享连接和锁；存储接口见 [store](../store/index.md)。
- [全站验收入口](../../scripts/acceptance/ui/main.js) 以 `nav.js` 注册页和 `ui/scope.js` 功能覆盖表为范围，校验成套构建、SPA 产物、四种屏宽、真实功能与 Server TUI；缺页、异常、超时或构建摘要不一致使验收失败。
- 验收 `--resume` 只复用覆盖范围及二进制摘要相同的成功记录，失败保留日志并重跑，SPA 漂移检查始终执行；`--brain-test` 可使用同批构建的测试可执行文件，其原生镜像版本检查仍然执行。
- [项目能力对接验收](../../scripts/acceptance/project/README.md) 的 `capabilities/main.js` 覆盖浏览器创建项目、专项和 TODO，连接 Agent、Operator、DAG、Brain，核验同一执行详情、实时结果读取、完整结果绑定与看板进度；隔离环境只替换模型传输，正式环境模式保留具名数据和版本回执。

## 相关
- [control](../control/index.md)、[brain](../brain/index.md) — Brain 校验/快照发布/执行在后端
- [动态 DAG 步骤](../../docs/dag-dynamic.md)
- [DAG 能力](../../features/dag/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
- [UI 验收约定](../../rules/05-ui-acceptance.md)
