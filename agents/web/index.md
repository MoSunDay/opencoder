Commit: 17ca79c3b55e06c4a2b677a16302a9ec1ed81b19

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

[access/useServerAuth.js](../../crates/web/spa/src/access/useServerAuth.js) 在连接服务器和凭据变化时读取公开鉴权状态；关闭时无需 Token 即可读取 admin 身份并打开控制台，JSON/SSE 请求和文件下载省略 Authorization。重新开启后 401 恢复登录入口。旧 Server 没有状态接口时沿用 Token 登录。

- 子项列表与编辑不显示状态；共享子项 API 拒绝 `status` 字段。TODO 看板状态和项目归档保持独立，子项进度只统计 TODO。

## 索引
- [admin/users.jsx](../../crates/web/spa/src/admin/users.jsx) — 创建用户后展示返回的初始 Token，支持复制；明文只在组件内存中保存，确认保存、离开页面或刷新后不再展示；编辑角色不签发 Token。列表与节点页共用 [admin/layout.jsx](../../crates/web/spa/src/admin/layout.jsx)；[admin/model.js](../../crates/web/spa/src/admin/model.js) 提供搜索与状态判断纯函数，[admin/formModal.jsx](../../crates/web/spa/src/admin/formModal.jsx) 管理单次编辑、提交互斥和失败草稿；[admin/styles.css](../../crates/web/spa/src/admin/styles.css) 使用 `oc-admin-` 前缀隔离后台样式。
- [chat/conversationInput.jsx](../../crates/web/spa/src/chat/conversationInput.jsx) — 会话页和 Agent/Operator 执行详情共用输入组件；按当前角色控制提交，保留引导、排队与问题交互。
- `src/lib.rs` — `AppState` 装配（`config_home`：Operator 执行 home，prompt/config 载入走 `Config::load_with_home`，drain 栈经 `DrainContext` 穿参）
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- [api_transcript.rs](../../crates/web/src/api_transcript.rs) — `GET /api/sessions/:id/transcript` 按消息序号和字节偏移读取完整展示消息块；TUI 经 Server 执行命令的 `http` 转发调用，复用已有渲染。
- `src/api_agents.rs`、`src/api_agent_resources.rs` — agent 目录/卡片与资源文件 API
- [api_dag_binaries.rs](../../crates/web/src/api_dag_binaries.rs)、[api_dag_binaries_nfs.rs](../../crates/web/src/api_dag_binaries_nfs.rs)、[api_dag_workspace_nfs.rs](../../crates/web/src/api_dag_workspace_nfs.rs) — 二进制版本池与独立只读导出管理；控制面和资源服务共享处理器，上传请求使用统一体积上限。
- [handle.rs](../../crates/web/src/handle.rs)、[handle/drain.rs](../../crates/web/src/handle/drain.rs) — `SessionHandle` 与 drain 生命周期；后台任务显式继承 Agent 资源根和执行技能根，Operator 从 `config_home` 解析冻结技能目录。嵌套任务作用域中的会话 future 使用堆分配。
- [handle_lifecycle.rs](../../crates/web/src/handle_lifecycle.rs) — SSE 最后一个订阅者离开时，持生命周期锁核验同一 handle、订阅数为零且没有 drain 后回收；不区分订阅者是否创建 handle。
- `src/auth_mw.rs`、`src/html.rs` — Bearer → Identity，独立指标凭据仅接受 `GET /metrics`；SPA 产物内嵌与 `/static` 白名单
- `src/api_control.rs` — 节点对话 API
- `spa/src/` — React18+antd SPA（vitest）；`src/html.rs` 在编译时嵌入已提交的 `spa/dist`，修改页面后须重建产物
- [theme.js](../../crates/web/spa/src/theme.js) 只维护 antd 配置；[theme.test.js](../../crates/web/spa/src/theme.test.js) 直接核对 CSS 声明与共享 token，并扫描生产源码中的变量引用。
- [harness/management.jsx](../../crates/web/spa/src/harness/management.jsx)、[configuration.js](../../crates/web/spa/src/harness/configuration.js) — Codex 默认配置与命名档案编辑模型和外部启动脚本；每行对应一个命令参数，空白表单保存为空数组。
- [SPA 产物检查](../../scripts/check-spa-drift.sh) — 在临时目录用 SPA 源码与包内资源重建，逐文件比较 `dist`；不复制仓库示例目录，不重试掩盖差异。
- `spa/src/chat.jsx`、`spa/src/chatSidebar.jsx`、`spa/src/chat/` — 会话页（Operator/Agent 双模式 lane）；Operator 创建前可选 Codex Harness 与逐行 env，随 `/api/sessions` 创建请求发送，启动后固定。`app.css` 在窄屏将会话侧栏与输入区纵向排列，保持输入区可操作；[modelModal.jsx](../../crates/web/spa/src/modelModal.jsx) 负责会话模型选择与切换。
- [chat/useTranscriptStream.js](../../crates/web/spa/src/chat/useTranscriptStream.js)、[chat/stream/completion.js](../../crates/web/spa/src/chat/stream/completion.js) — `sse.js` 的 `onEnd` 单独通知 `stream_end`；Hook 在 15 秒内读取执行结果与会话快照，纯函数合并消息与结束状态。读取失败恢复输入；取消或重新订阅使旧读取失效，包括同一会话的新订阅。
- [chat.jsx](../../crates/web/spa/src/chat.jsx) — 打开运行中或初始化中的会话时先读游标，再取快照并续接事件；创建重试只在首轮输入不变时复用原 ID。Agent 配置不提供容器执行开关；启动取消和中断显示实际原因。
- `spa/src/main.jsx` — 身份确认完成后才挂载导航与页面；身份格式错误和读取失败提供重试，401 返回登录入口。
- [nav.js](../../crates/web/spa/src/nav.js)、[shell/categoryTabs.jsx](../../crates/web/spa/src/shell/categoryTabs.jsx) — 项目、Agent、Ontology、后台管理四类导航；标签保持完整宽度，容器支持滚轮、触摸和键盘滚动，并保持当前标签可见。editor 与 viewer 可打开平台页面，后台管理仅管理员可见；viewer 的写入控件禁用。项目 Overview 登记为 `body-title`，从项目、子项、TODO 页签开始，PageShell 不生成额外标题行。用户权限登记为 `menu-only`，页面从表格工具栏开始，不渲染额外的标题和说明；不设独立 Token 管理页。
- [Ontology 前端](ontology.md) — 四个页面固定使用当前 Server 的默认数据空间，提供图谱与结果列表、实体详情草稿及观测记忆；复用平台身份、请求与 antd。接口与存储见 [ontology](../ontology/index.md)。
- `spa/src/ui/requests/query.js` — 读取请求的取消、迟到响应丢弃、响应校验与错误状态；失败不替换为空数据。
- `spa/src/fleet/`、`spa/src/schedule/` — 执行表与定时任务页；`schedule/history.jsx` 按历史记录的执行 ID 打开原执行，不重新派发。节点调度设置读取失败时禁止保存默认值。
- [fleet/detail.jsx](../../crates/web/spa/src/fleet/detail.jsx)、[fleet/detail/workloads.jsx](../../crates/web/spa/src/fleet/detail/workloads.jsx) — TODO 执行明细在工作流建立后加载工作台；初始化、停止和初始化失败只展示对应状态，初始化错误只显示一次。工作流建立后的读取错误仍显示实际原因。
- [fleet/teams/](../../crates/web/spa/src/fleet/teams/) — Team 列表、成员编辑、启动表单与可复用的 `TeamExecutionHistory`；历史记录按服务端游标分页并自动刷新，点击打开原执行。`submission.js` 限制提交与回执读取的等待时间并校验执行身份，`useSubmission.js` 在提交超时或回复不确定后查询现有 `/receipt`，确认受理后跳转详情。重试保留原请求，关闭时取消读取并忽略迟到响应。
- [fleet/detail/team/](../../crates/web/spa/src/fleet/detail/team/) — 通用执行详情复用 `TeamExecutionProcess`，展示轮次、发言与小结；`progress.js` 读取当前未完成轮次已落盘的计划与小结，`record.jsx` 更新尚未生成的发言。读取有界、请求不重叠，澄清对象来自上次小结；切换执行取消旧请求，大记录保留分段读取，异常结束保留已保存的未完成轮次。
- `src/api_project*.rs` — 项目、子项、TODO 与 Tag 的共享 HTTP 处理器；Tag 范围和选择经存储验证，TODO 顺序写入携带 `initiative_id` 范围，负数位置保留给迁移且 API 拒绝；Control 复用同一组处理器
- [api_project_people.rs](../../crates/web/src/api_project_people.rs)、[project/people/fields.jsx](../../crates/web/spa/src/project/people/fields.jsx) — 只读用户名目录、人员错误映射与三层共用的选择、展示和筛选；请求失败提供重试，按原有角色禁用写入，清空负责人使用显式 `null`。
- `spa/src/project/`、`views/projectTable.jsx`、`views/viewState.jsx` — 项目、子项、TODO、Tag 四个表格与列筛选，视图状态在保存刷新及抽屉关闭后保留；项目和子项分别进入 `views/projectDrawer.jsx`、`views/initiativeDrawer.jsx`；项目详情在桌面使用 `size="70vw"`，1024px 及以下全宽；子项详情与 `todoDrawer.jsx` 使用 `size="100vw"`，内部容器不再限宽居中。
- [project/navigation/links.js](../../crates/web/spa/src/project/navigation/links.js) 编解码 `project/initiative/todo` 定位，优先片段、兼容查询参数；[project.jsx](../../crates/web/spa/src/project/project.jsx) 通过 [hierarchy.js](../../crates/web/spa/src/project/navigation/hierarchy.js) 从地址目标及总览中的实际归属恢复父级，逐层打开和关闭写入浏览器历史。同一时刻只打开最深层抽屉，父级组件保留当前页签；关闭按钮与 Escape 均只返回一层，避免异步加载和门户挂载顺序导致错层。目标 ID 从总览投影查找，不依赖当前筛选或父级 ID；[unavailable.jsx](../../crates/web/spa/src/project/navigation/unavailable.jsx) 分别展示加载中、读取失败和记录不存在。
- [project/forms/](../../crates/web/spa/src/project/forms/) — TODO 共用一个 Ant Design Form，任务与设置页签字段预先注册，切换保留值；MarkdownInput 通过 defaultMode 区分新建编辑和已有正文预览；只切换说明区域，标题与状态输入框同行并始终挂载，任务页底部的评论独立维护草稿与提交，三个页签切换保留各自内容。标题校验回到任务页并聚焦，保存失败保留草稿；关闭未保存内容由 Modal 确认。创建使用任务/设置；项目详情保留概览/子项页签，子项详情将现状与说明、基本信息与进度、看板依次展示，基本信息和进度靠左相邻排列。真实 CRUD 和四种屏宽验收见 [project/forms.js](../../scripts/acceptance/project/forms.js)。
- [project/summary/view.jsx](../../crates/web/spa/src/project/summary/view.jsx)、[views/mdDrawer.jsx](../../crates/web/spa/src/project/views/mdDrawer.jsx) — 项目、子项列表与详情共用现状及时间展示，沿用表格筛选。子项表单将 detail_md 明确标为“子项目标”并前置，表单保留打开时的现状基线，只提交被编辑的现状，避免改其他字段时更新时间或覆盖刷新后的新现状。共享项目 API 为显式现状写入生成时间。[history.jsx](../../crates/web/spa/src/project/summary/history.jsx) 复用于详情及三个列表的现状历史按钮，阻止点击穿透到行详情；按需读取版本，处理错误、取消过期读取并确认恢复；[现状 API](../../crates/web/src/api_project/summary.rs) 复用读写权限，返回最近 10 个版本及恢复来源。
- [project/sorting/](../../crates/web/spa/src/project/sorting/) — 项目内子项使用表格手柄拖拽及键盘排序；纯函数在完整列表上移动，保留筛选隐藏项，保存中禁用重复操作，失败恢复原顺序。排序接口携带完整原顺序和目标顺序。子项名称列与所属项目等宽，名称换行展示。
- [tags/panel.jsx](../../crates/web/spa/src/project/tags/panel.jsx)、[tags/tab.jsx](../../crates/web/spa/src/project/tags/tab.jsx)、[tags/editor.jsx](../../crates/web/spa/src/project/tags/editor.jsx) — Tag 独立菜单及管理表格；Overview 保留项目、子项、TODO 三个页签。定义行由 `tags/model.js` 纯投影，支持全局、项目和子项；全局隐藏归属选择，切换类型清空旧归属，保存失败保留草稿。共享 [api_project_tags.rs](../../crates/web/src/api_project_tags.rs) 校验名称与归属，全局允许省略或传空 `scope_id`，修改保留 ID。
- `spa/src/project/board/`、`model/board.js`、`model/catalog.js` — dnd-kit 看板与纯移动、进度、Tag 解析；卡片将标题、可读说明摘要、标签、负责人分层展示，标题与卡片内容直接拖拽，更多菜单阻止拖拽冒泡；标题保留 Enter 打开、空格开始键盘排序，Escape 取消。按完整任务集合计算筛选后的拖动顺序，多 Tag 卡片共享 TODO ID，失败回退原数据。
- [project/comments/](../../crates/web/spa/src/project/comments/) — Markdown 评论、认证用户署名、来源跳转和稳定请求 ID；失败保留正文和 ID，分页去重并保持滚动位置，可见时读取全部增量页，隐藏时取消读取并隔离迟到响应。
- [project/todoDrawer.jsx](../../crates/web/spa/src/project/todoDrawer.jsx)、[project/execute/](../../crates/web/spa/src/project/execute/) — TODO 选择实际能力 ID，以稳定执行 ID 派发并关联；回复丢失时保留同一请求重试。执行页可见时读取执行，任务页可见时读取内嵌评论，隐藏时停止轮询；能力选择和指派按钮同行显示，网页不提供手动关联输入，派发后的自动关联及失败重试保留；执行列表只展示关联记录和状态，不内嵌执行结论，执行 ID 通过新页面链接进入原生执行详情；[fleet/navigation/links.js](../../crates/web/spa/src/fleet/navigation/links.js) 编解码执行 ID 与类型，Shell 的首次页面选择优先执行链接，ExecutionsPanel 无需在当前列表页找到该记录即可挂载原生详情；评论来源与派发后的内嵌 `ExecutionView` 保留原会话、事件与引导入口。
- [chat/inputAttempt.js](../../crates/web/spa/src/chat/inputAttempt.js) — 会话页与执行明细共享人工输入 ID 的生成与重试规则，未确认的回复保持原输入 ID，避免断线重试重复提交。
- `spa/src/dag/editor/canvasEditor.jsx` — 用已发出的 spec 签名避免重复通知，并保证依赖边更新在节点编辑之后仍能保存
- [fleet/teams.jsx](../../crates/web/spa/src/fleet/teams.jsx)、[fleet/teams/](../../crates/web/spa/src/fleet/teams/) — Team 列表、成员编辑、启动及执行历史；成员以 Agent 身份配置。
- [agentDetail.jsx](../../crates/web/spa/src/agentDetail.jsx)、[agents/](../../crates/web/spa/src/agents/) — 当前 Agent 资源编辑入口，统一读取和保存 Prompt、Skills、Tools、Memory；读取失败禁止覆盖，切换页签保留草稿。NFS 状态由 `agentNfsCard.jsx` 展示五类实际导出。
- `spa/src/dag/resources/` — 二进制池界面；`model.js` 负责 ELF 与版本引用纯校验，`read.js` 校验池和历史响应，`editor.jsx` 发布文件，`panel.jsx` 管理下载、删除与当前版本指针，`field.jsx` 为步骤选择受理时 current 或固定版本。
- `spa/src/dag/` — DAG 定义、运行、原生步骤与动态实例页面。图投影在 `dagProjection.js`，快照与后续事件合并在 `run/model.js`，编辑器在 `editor/`；布局与依赖变更由组件测试验证，执行约定见 [规则 04](../../rules/04-dag-execution-contract.md)。
- `spa/src/dag/specValidate.js` 与 `editor/canvasModel.js` — 校验定义并在画布编辑中保留 `max_concurrency` 等字段；运行使用已冻结的定义快照。
- `spa/src/dag/dynamic/`、`spa/src/brain/workbench/` — 动态 DAG 与 Brain 工作台
- [dag/step/binaryLogs.jsx](../../crates/web/spa/src/dag/step/binaryLogs.jsx) — 原生二进制步骤输出；编辑器只提供 Binary、Agent 与 Dynamic，运行页面按节点所属执行读取实例、日志与声明产物。
- [dag/run/context.jsx](../../crates/web/spa/src/dag/run/context.jsx) — DAG 与执行明细共用的只读运行环境；容器、工作目录、版本和摘要来自节点保存的运行资料，不从当前资源池推测历史。
- `spa/src/brain/workbench/` — schema 7 工作台。`scheduler/editor.jsx` 在 `milestone/` 画布上配置必填里程碑信息和绑定泛化能力的并行节点，随后用表单提交计划信息；节点名称和任务由能力库生成。画布按顺序展示相邻层，大脑在运行时决定是否回到已执行层。`milestone/run.jsx` 管理画布与详情抽屉，`milestone/runtime/viewport.js` 计算可用高度；`runtime/model.js` 从全部历史操作中选择层或节点的最新执行，`runtime/layerDetails.jsx` 展示单任务、多任务与历史记录，复用 `ExecutionView`。`runDetails.jsx` 保留按轮次汇总的详情表格；右侧抽屉中的人工输入，以及受计划管理的 Agent/Operator/Team 明细引导，统一提交到 Brain 输入事件；托管明细不暴露单独中断/取消执行的控件。`useRun.js` 读取 `/layered`，由事件流及轮询刷新。
- [milestone/canvas.jsx](../../crates/web/spa/src/brain/workbench/milestone/canvas.jsx)、[canvas/presentation.js](../../crates/web/spa/src/brain/workbench/milestone/canvas/presentation.js) — 节点成员与内容只由计划投影；本地状态只保留选择、测量和拖动过程。位置以增量写入草稿，React Flow 不直接删除计划节点。运行画布按可用宽度排列层级，编辑器保持纵向布局。
- [scheduler/changes.js](../../crates/web/spa/src/brain/workbench/scheduler/changes.js)、[scheduler/draft.js](../../crates/web/spa/src/brain/workbench/scheduler/draft.js) — 编辑动作应用到同步保存的最新草稿，持久化读取同一份最新值；批量删除按层清理子节点及该层位置，迟到的位置事件不能恢复已删层。[scheduler/editor.jsx](../../crates/web/spa/src/brain/workbench/scheduler/editor.jsx) 统一按钮与键盘删除确认，只有显式整理布局才清空位置。
- [brain/workbench/plans.jsx](../../crates/web/spa/src/brain/workbench/plans.jsx) — 新建计划、点击当前计划名称与修改操作共用 `PlanEditor`；历史版本预览独立打开，旧格式必须显式转换后编辑。读取请求用序号隔离，迟到响应不能覆盖新建表单。
- [brainPanel.jsx](../../crates/web/spa/src/brainPanel.jsx)、[brain/library.css](../../crates/web/spa/src/brain/library.css) — 能力表格管理当前页与每页条数；刷新、保存保留分页选择，搜索重置页码，删除后收敛到有效页。列表与搜索结果按能力 ID 关联工作台已加载的 `/api/brain/library`，展示实际绑定的执行类型与名称；刷新、保存和删除同时更新绑定目录，读取失败独立报错，过期请求不能覆盖新结果。类型与名称在本列完整换行，一句话描述限宽省略；手机上的操作列随表格横向滚动。输入、输出与工程示例由 `CapabilityEditor` 读取完整详情后展示。
- `tests/` — 集成测试

## 接缝

- [fleet/navigation/location.js](../../crates/web/spa/src/fleet/navigation/location.js) 监听地址和浏览器历史；执行地址持续优先于持久化的主页选择。[links.js](../../crates/web/spa/src/fleet/navigation/links.js) 生成仅含执行 ID 的片段链接，兼容旧查询参数，并在关闭详情或显式切换菜单时清除执行定位。TODO 不依赖预先取得类型或列表第一页；ExecutionView 按 ID 请求 Server 原生详情，再由返回类型展示能力过程。
- [fleet/executions.jsx](../../crates/web/spa/src/fleet/executions.jsx) 直接按地址打开和关闭执行详情；名称等摘要取自当前列表，不再保存一份选中记录。列表没有该执行时仍按 ID 读取原生详情。
- 项目定位同样优先于持久化主页，显式切换菜单清除定位。[fleet/navigation/history.js](../../crates/web/spa/src/fleet/navigation/history.js) 给站内历史条目标记顺序；[DraftGuard](../../crates/web/spa/src/ontology/navigation/DraftGuard.tsx) 统一注册未保存或提交中的表单，项目 [closeGuard.js](../../crates/web/spa/src/project/forms/closeGuard.js) 接入同一保护。浏览器返回时先恢复原条目再确认，确认前固定地址快照，避免异步界面更新提前卸载草稿；取消保留草稿与地址，确认后继续原方向，整页离开沿用 beforeunload。导航验收见 [navigation.js](../../scripts/acceptance/project/navigation.js)。

- 原生执行详情与消息翻页兼容缺少 `Array.at`、`URLSearchParams.size` 的浏览器；会话 Markdown 无法解析时以普通文本展示，仍可读取 Session 上下文。执行深链测试挂载真实 `ExecutionView`，检查所属节点、消息、分页和离线错误。
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。
- [serve](../../crates/web/src/lib.rs) 为会话与 `ProjectService` 提供同一 `LibsqlStore` 实例，共享连接和锁；存储接口见 [store](../store/index.md)。
- [项目工作台浏览器步骤](../../scripts/acceptance/project/browser.js) 通过当前子项与 TODO 入口创建层级；关闭关联详情时依次退出实际打开的抽屉，再回到 TODO 列表发起下一次操作。
- [UI 验收入口](../../scripts/acceptance/ui/main.js) 与 [范围表](../../scripts/acceptance/ui/scope.js) — 按 [规则 05](../../rules/05-ui-acceptance.md) 选择受影响页面和行为；全站验收仅在用户明确要求时执行。
- [远端会话验收](../../scripts/acceptance/conversations/README.md) 共用节点选择、会话身份核对与回复等待，验证两种模式的只读工具调用、续聊和重开。
- 验收 `--resume` 只复用覆盖范围及二进制摘要相同的成功记录，失败保留日志并重跑，SPA 漂移检查始终执行；`--brain-test` 可使用同批构建的测试可执行文件，其原生镜像版本检查仍然执行。
- [responsive/services.js](../../scripts/acceptance/ui/responsive/services.js) 直接绑定系统分配的空闲端口并返回实际端口，避免先查询再占用的竞争；显式端口冲突返回启动错误。 [响应式验收夹具](../../scripts/acceptance/spa_responsive_fixtures.js) 包含页面启动时的 `/api/auth/status` 读取；未知 API 请求仍使验收失败。
- [大脑运行画布验收](../../scripts/acceptance/brain/ui/run-canvas.js) — 只读打开已有运行，在四种屏宽核对画布尺寸、全屏和每层实际执行详情；可用本地候选页面代理真实 API。
- [大脑编辑画布验收](../../scripts/acceptance/brain/ui/editor.js) — 对实际 SPA 产物执行键盘及批量删除、取消、拖动、连续修改、关闭重开和新版本保存，核验计划层级与位置；使用临时 HTTP 数据，不操作已有计划。
- [项目能力对接验收](../../scripts/acceptance/project/README.md) 的 `capabilities/main.js` 覆盖浏览器创建项目、子项和 TODO，连接 Agent、Operator、DAG、Brain，核验同一执行详情、实时结果读取、完整结果绑定与看板进度；隔离环境只替换模型传输，正式环境模式保留具名数据和版本回执。

## 相关
- [control](../control/index.md)、[brain](../brain/index.md) — Brain 校验/快照发布/执行在后端
- [动态 DAG 步骤](../../docs/dag-dynamic.md)
- [DAG 能力](../../features/dag/index.md)、[执行约定](../../rules/04-dag-execution-contract.md)
- [UI 验收约定](../../rules/05-ui-acceptance.md)

[project/summary/section.jsx](../../crates/web/spa/src/project/summary/section.jsx) 为项目、子项详情的现状和目标提供独立折叠按钮，默认收起正文，标题、现状时间与历史入口始终可用。记录 ID 作为组件 key；刷新同一记录保留展开状态，切换记录重置。网页名称“子项 / 目标 / 现状”分别对应 `initiative / detail_md / current_summary`。

TODO 正文显示为“关键信息”，仍使用 draft；详情新增文件页签，单独上传、下载、确认删除，切换时保留任务和评论草稿。Agent 配置页显示第五路 TODO 文件只读导出。见 [任务文件](../../docs/project-files.md)。
