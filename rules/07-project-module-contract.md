Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# 规则 07：项目模块约定

修改项目、专项、TODO 的数据关系、看板、标签、执行关联、结果读取或进度计算之前，先读本规则。核心职责是：**项目承载整体目标，专项组织某一方向的工作，TODO 记录具体要完成的事情，执行记录保存实际执行过程和结果。**

功能入口见 [项目工作台](../features/project/index.md)，实现索引见 [project 模块](../agents/project/index.md)。涉及大脑调度时同时遵守 [规则 06](06-brain-scheduling-contract.md)，涉及 DAG 执行时同时遵守 [规则 04](04-dag-execution-contract.md)。

网页将专项显示为“子项”，项目与子项的“说明”显示为“目标”，“当前概述”显示为“现状”。对应字段仍为 `initiative`、`detail_md`、`current_summary`；目标和现状在详情中独立折叠，默认收起正文，现状时间和版本入口保持可见。

## 对象、归属与职责

| 对象 | 职责 | 当前主要信息与状态 |
|------|------|--------------------|
| 项目 | 承载整体目标、说明与总体进度 | 名称、说明、排序；`active` 进行中、`archived` 已归档 |
| 专项 | 组织某一方向的一组工作 | 名称、说明、可选所属项目、排序；无自身状态，进度由 TODO 看板状态计算 |
| TODO | 跟踪具体任务及交付结果 | 标题、关键信息、可选所属专项、看板列、位置、标签、执行方式和指派历史 |
| 执行记录 | 记录为 TODO 发起或关联的一次执行 | 能力 ID、执行 ID、执行类型、名称和创建时间；状态与结论来自所属执行节点 |

- 层级是 TODO → 可选专项 → 可选项目。一个项目可含多个专项，一个专项可含多个 TODO；每个专项最多归属一个项目，每个 TODO 最多归属一个专项。
- 专项可以独立存在，TODO 可以未归属专项；TODO 通过专项确定所属项目，不直接挂载项目。新建或调整归属时须验证目标存在。
- 项目在代码和 API 中使用 `goal`，专项使用 `initiative`；不能因为内部名称不同再增加一套并列对象。
- 项目、专项和 TODO 负责组织与跟踪工作。任务拆分、保存和指派由用户或用户明确授权的 Operator 发起；保存关键信息不自动解析能力、创建执行或展开大脑计划。
- 一个 TODO 可以关联多次执行，每次保留独立执行引用。不能将 TODO 等同于某个 Agent 会话、某次 DAG 运行或某次 Brain 激活。
- 项目里程碑入口已移除，存量里程碑下的 TODO 已转为未归属任务；不能重新加入旧里程碑层级。大脑计划内的里程碑层属于执行编排，与专项没有自动的一一映射。

## 从任务到执行的流程

1. 创建或编辑 TODO，保存标题、关键信息、归属、标签和看板状态。
2. 选择能力库中的实际能力 ID，填写任务与命名参数。Agent、Operator、Team、DAG、TODO 工作流及已保存 Brain 计划复用原生执行入口。
3. 通过 `POST /api/project/todos/:id/dispatch` 派发，携带稳定的执行 ID。Fleet 回执固定能力和输入，重试复用原执行，不重复创建任务。
4. TODO 保存执行关联；API/CLI 仍可关联已有执行 ID，网页不再提供手动粘贴输入。关联接口核验实际执行和准入能力，重复关联幂等。已受理但关联失败时保留执行 ID，可重试关联。
5. TODO 执行列表只展示关联记录和状态；点击执行 ID 打开原生执行明细查看结论。过程、消息、步骤及完整产物从同一原生执行明细查看。

每次指派保留独立执行引用；新指派不覆盖旧指派。解除关联只移除引用，不取消或删除原执行。

## 看板状态与执行状态

- TODO 看板列固定为 `backlog` 待整理、`todo` 待办、`in_progress` 进行中、`done` 已完成，由用户编辑、拖动或明确授权的 Operator 在验收后维护。
- 执行状态来自原生执行索引。执行结束或取得结果均不能自动将 TODO 标为已完成，也不能作为任务验收通过的依据。
- 项目执行关联只保存 TODO ID、执行 ID、能力 ID、类型、名称和创建时间。不保存结论副本或同步状态，不启动后台结论收集，不提供重新收集接口。
- 查询失败显示实际错误；节点离线返回不可用。界面清除旧的实时结果并提供重试，不能将旧内容当作当前执行结果，也不能因节点离线修改看板状态。
- 原生执行明细由执行 ID 和所属节点定位；项目页面不另建执行状态机。点击链接只需要执行 ID，执行类型与原始归属由 Server 解析，不要求当前索引可读或列表包含该记录，也不能因恢复主页选择而丢失执行定位。运行中的 Team 可通过原生详情提交引导。
- 旧 `ProjectTodoStatus` 的 `draft/planned/running/done/failed` 属于旧项目执行路径；当前看板、进度和用户编辑以 `board_status` 为准。

## 结果来源与边界

- Agent、Operator 使用节点保存的助手输出正文；Team 使用最终总结；TODO 工作流读取候选结果；Brain 使用调度运行总结。
- DAG 的完整输出保留在步骤明细和产物接口；执行结论入口提供摘要和步骤入口，不把部分步骤结果冒充整次运行结果。
- 实时结论有展示上限；超长或省略内容必须明确提示从原生明细读取完整结果。尚未提供结论、读取失败与执行失败须分别展示。
- 完整结果、消息、步骤和产物始终由所属执行节点持有。历史执行沿用原 Runtime 归属，读取不触发迁移或改写执行。
- 项目和会话使用同一 Turso 实例；schema 33 的关联表只保留执行引用。不得重新引入已移除的结论缓存字段、后台回写流程或可选 SQL 后端。

## 当前概述

- 项目和专项保存 `current_summary` 与 `summary_updated_at`，分别表示当前人工或 Operator 编写的概述、概述提交时间。概述按关键进展、核心工作、剩余卡点组织；不作为 TODO 验收依据。
- 专项通过 ctl `project initiatives summary-context` 读取全部 TODO、全部执行关联及原生过程证据后整理；项目通过 `project goals summary-context` 汇总所属全部专项，先补齐缺失或过时的专项概述。
- 概述通过已有创建/修改接口写入，服务器记录时间。PATCH 不传正文保持原概述和概述时间，空字符串清空；其他字段修改不刷新概述时间。Web 列表与详情显示概述及其时间。
- 概述保存最近 10 个版本（含当前版本），初始内容、清空和回滚均留版本记录。Web 与 ctl 可查看版本内容、时间及回滚来源；回滚新增版本并更新概述时间，不重写原版本。正文、历史与超额清理在同一事务完成，已清理版本返回不存在。查看沿用读权限，回滚沿用写权限；删除对象清理自身概述历史。
- 概述属于用户授权维护的项目说明。原生执行结果与过程仍在节点，仅按需读取；不缓存到执行关联，不启动后台收集或自动修改看板。证据读取失败列出来源与原因，不视为完整证据；运行中的执行仍可能产生新进展。

## 进度计算

- 专项进度＝该专项中 `board_status = done` 的 TODO 数 ÷ 该专项 TODO 总数。
- 项目进度＝所属全部专项中 `board_status = done` 的 TODO 数 ÷ 所属全部专项 TODO 总数；不能简单平均专项百分比。
- 无 TODO 时显示 0%；未归属专项的 TODO 不计入任何专项或项目，独立专项的 TODO 不计入项目。
- 多标签分组造成的重复展示只计同一 TODO 一次；筛选后的可见卡片数量不能替代整体进度的分母。
- 专项不保存、返回或接受自身状态，创建和编辑界面不提供状态，API 拒绝 `status` 字段。进度只来自 TODO 的 `board_status`；进度达到 100% 不自动归档项目。

例：专项 A 有 1 个 TODO 且已完成，专项 B 有 9 个 TODO 且均未完成，所属项目进度为 10%。

## 标签与页面操作

### 负责人和关注人

- 项目、专项、TODO 各自保存可为空的单个 `owner_user_name` 和去重后的 `follower_user_names`。只能引用现有 OpenCoder 用户；三层独立，不继承、不自动复制，移动归属不改变人员。
- PATCH 未传人员字段时保持原值；负责人传 `null`、关注人传 `[]` 分别清空。负责人也可关注同一对象。人员校验与对象、Tag 修改须在同一事务中完成，非法用户不能留下部分成功修改。
- 人员关系统一存储，删除对象清理自身关系，删除用户清理其关系；重新创建同名用户不能继承旧关系。旧项目升级后的默认值为空，不修改既有鉴权数据。
- `GET /api/project/people` 只向已认证用户返回可选用户名，不返回角色、Token 等信息。沿用项目写权限，关注关系不授予额外编辑权限。
- 三层列表支持人员展示和筛选，创建与编辑支持设置、清空。人员关系作为后续通知数据源；本轮不发送通知，也不增加通知订阅、收件人继承或投递状态。

### 标签定义与视图

- 项目模块的独立 Tag 表格统一管理标签，可创建、修改名称和全局、项目或专项归属、删除；全局 Tag 的 `scope_type=global`、`scope_id` 为空字符串，其他类型归属一个项目或专项。修改后保留 ID，同一范围内名称唯一。同名时依次使用专项、项目、全局定义。
- 一个 TODO 可选多个当前归属范围内的 Tag；全局 Tag 可用于所有项目、专项和未归属 TODO，未归属专项的 TODO 只能选择全局 Tag，不能接受其他范围的标签关联。
- 改变归属、覆盖或删除标签定义后，按名称匹配新的可用标签；有同名定义则接替，没有则清除失效关联。同时修改标签名称和归属时按保存后的名称匹配；未选择标签的 TODO 不自动添加标签。无效标签选择不能留下部分成功的任务或关联修改。
- 按 Tag 分组时，一个 TODO 可展示在多个组内；状态变化须同步到各处展示，保存的仍是同一条 TODO。
- 项目分类下依次提供 Overview、Tag 两个菜单入口；Overview 保留项目、专项、TODO 三个表格页签，Tag 独立管理全局、项目和专项的各条标签定义，优先级用于实际标签选择。项目详情在桌面以 70% 视口宽度展示概览与专项，窄屏全宽；专项详情不使用页签，先展示当前概述与专项说明，再展示靠左相邻排列的所属项目、人员和进度，最后展示 TODO 看板；编辑专项明确提供专项说明 Markdown 字段。专项和 TODO 内容左对齐，使用抽屉可用宽度。TODO 详情按任务、文件、设置、执行切换，状态选择放在任务标题输入框右侧，两个字段在同一行显示并由底部保存，评论放在关键信息底部；执行能力选择与指派按钮同行显示；指派记录点击执行 ID 在新页面打开对应原生执行详情，不显示查看按钮，不丢弃原任务与评论草稿。说明默认预览 Markdown，直接切换说明的预览/编辑，统一用底部“保存 TODO”提交，不提供整页“编辑任务”入口；标签管理统一从 Tag 菜单操作。
- 项目内专项支持拖拽排序，复用现有排序字段；提交完整原顺序和目标顺序，事务内核验项目归属、成员集合及原顺序，冲突拒绝保存，不能部分更新。排序不刷新概述时间或追加概述版本。
- TODO 卡片不提供单独拖动按钮，标题和内容可直接拖动；点击标题打开详情，更多菜单不触发拖拽。键盘在标题上用空格开始排序、Escape 取消，取消不得关闭专项详情。
- 筛选条件在刷新、保存和关闭后重新打开详情时保留。筛选后拖动须保留隐藏卡片的相对顺序；保存失败恢复原看板状态。
- TODO 总表包含所有归属和未归属任务；已有悬空关系在概览中展示为独立专项或未归属 TODO，不能静默丢失。
- 项目、专项和 TODO 详情使用 `project`、`initiative`、`todo` 地址参数定位，优先片段、兼容查询参数。分享只需目标 ID，不能要求先打开父级或在当前筛选结果中找到记录；打开、关闭、刷新与浏览器前进后退同步详情。登录和保存的主页选择不得丢失定位。未保存草稿离开前须确认，取消保留原内容和地址；读取失败不能显示为记录已删除。
- 仅含目标 ID 的分享链接也按总览中的当前归属恢复完整父级；关闭 TODO 回到所属专项，再关闭专项回到所属项目。独立专项和未归属 TODO 不生成父级，过时的父级参数不能覆盖当前归属；父级异步加载不得遮挡目标详情。

### TODO 评论

- editor 只能修改或删除自己提交的评论，admin 可管理全部评论，viewer 只读；能力署名评论以实际提交用户判定权限。编辑和删除必须提供当前 `expected_revision`，过期返回 409；删除须确认，不修改其他评论。

- TODO 支持追加、编辑和删除 Markdown 评论：网页由当前用户署名，API/CLI 可使用全部六类注册能力 ID 署名，同时保存实际提交用户和名称/类型快照。沿用项目权限，不增加能力登录身份；历史署名不因名称、用户或能力变化被改写。
- 可选执行来源必须已关联当前 TODO，能力署名必须与真实准入能力相符；事务追加时再次核验关联。正文最大 64 KiB，按 TODO、提交用户及请求 ID 幂等，相同键不同内容返回 409。编辑只修改正文，保留序号、创建时间、署名与来源；原追加请求重试返回当前正文，已删除评论的原请求返回 409，不能重新创建。
- 评论在任务页正文底部展示，使用独立递增序号分页，最近页默认 50 条；任务页可见时刷新已加载范围以同步编辑与删除，隐藏时停止并取消读取。新增、编辑、删除更新 TODO 更新时间，专项 summary-context 读完全部评论并明确列出失败来源；不修改看板，不作为任务验收或执行结论。
- 页签切换、异步读取和后台刷新不得丢弃编辑或评论草稿，关闭未保存内容须确认；保存 TODO 与发表评论分别提交。读取到已有执行不得自动抢走任务页。

## TODO 过程文件

- TODO 的关键信息沿用 `draft` Markdown 字段，用于持续同步目标、进展、结论、卡点与下一步；不自动生成或保留正文版本。
- 用户或 Agent 通过 Server HTTP/CLI 主动上传任务文件，专用 NFS 只读共享。目录固定为 TODO 创建时间的 UTC 日期及 ID；原生执行结果仍由执行节点持有，不自动收集或作为结果缓存。
- 页面支持上传、逐个下载和确认删除，独立于任务保存与评论；有文件或无法核验文件时，删除 TODO 返回错误。只有明确清空文件后才允许删除任务。
- 文件发布与任务删除通过跨进程锁协调，完整上传后才原子发布，不隐式覆盖同名文件；备份同时覆盖实际文件根及绑定信息。接口、配置和边界见 [任务文件](../docs/project-files.md)。

## 删除与解除归属

- 删除项目时，保留其专项、TODO 和执行记录；专项解除项目归属，项目 Tag 按剩余可用定义重新匹配。
- 含 TODO 的专项不能删除，须先移动 TODO 或解除归属；不能通过删除专项连带删除任务。
- 删除 TODO 时，清理该任务的评论、标签关联、执行关联和旧 Project 运行记录。已关联的原生执行仍由执行系统管理，删除任务或解除关联不作为取消执行命令。

## 执行边界与实现依据

- 项目 TODO 是任务管理记录；TODO 工作流是可选的执行能力，内部有独立任务与运行状态，不能混用两者的 ID、状态或完成规则。
- TODO 交给 Brain 时选择已保存的计划版本并传入任务输入，关联整次 Brain 运行；Brain 的里程碑、节点和返工轮次留在该次运行内。其执行完成不改变项目 TODO 的看板状态。
- 执行关联与原生详情由 Server 控制台提供。独立 Web 的旧项目 API 不提供控制台执行索引，不能据此假定执行不存在。
- 旧 `ProjectService` 的 plan/execute 路径与历史记录仍有独立实现，当前项目页面不提供旧的专属计划、执行或回放入口；维护时不能把旧路径的状态规则套到看板指派链路。
- 项目数据通过 `ProjectStore` 持久化；关系投影、进度、标签选择与重匹配使用纯函数，API 和 UI 复用相同规则。

代码入口：[数据定义](../crates/store/src/project_types.rs)、[存储接缝](../crates/store/src/project.rs)、[概览与进度](../crates/store/src/project/overview.rs)、[标签规则](../crates/store/src/project/tags.rs)、[原生执行入口](../crates/web/spa/src/project/execute/launcher.jsx)、[执行关联](../crates/control/src/api/project_links.rs)、[实时结果](../crates/control/src/api/executions/results/mod.rs)。

## 变更验收

修改相关实现时，测试必须覆盖受影响的行为；现有用例入口：

| 范围 | 测试文件 |
|------|----------|
| 可选归属、删除保护、执行关联与历史数据 | [project_relations.rs](../crates/store/tests/project_relations.rs)、[project_store/suite_2.rs](../crates/store/tests/project_store/suite_2.rs) |
| 标签继承、覆盖、重匹配与失败回滚 | [project_tags.rs](../crates/store/tests/project_tags.rs) |
| 三层独立人员、空值、权限、迁移与删除清理 | [project_people.rs](../crates/store/tests/project_people.rs)、[人员 HTTP 测试](../crates/control/tests/e2e/project_api/people.rs)、[人员表单测试](../crates/web/spa/src/project/people/fields.dom.test.jsx) |
| 六类执行关联、能力核验与幂等 | [project_links.rs](../crates/control/tests/e2e/project_links.rs) |
| 实时结果、离线错误与执行切换隔离 | [原生执行明细](../crates/web/spa/src/fleet/detail/liveTranscript.dom.test.jsx)、[结果接口](../crates/control/tests/e2e/project_links/dispatch.rs) |
| 看板完成数与项目加权进度 | [overview.rs](../crates/store/src/project/overview.rs) 内联测试、[catalog.test.js](../crates/web/spa/src/project/model/catalog.test.js) |
| 归属编辑、看板移动与筛选保留 | [归属表单](../crates/web/spa/src/project/forms/editor.dom.test.jsx)、[relations.dom.test.jsx](../crates/web/spa/src/project/views/relations.dom.test.jsx)、[board.test.js](../crates/web/spa/src/project/model/board.test.js)、[project.dom.test.jsx](../crates/web/spa/src/project/project.dom.test.jsx) |
| 详情分享、登录后定位、历史切换与草稿保护 | [navigation.dom.test.jsx](../crates/web/spa/src/project/navigation/navigation.dom.test.jsx)、[history.dom.test.jsx](../crates/web/spa/src/fleet/navigation/history.dom.test.jsx)、[浏览器验收](../scripts/acceptance/project/navigation.js) |
| 项目内专项排序、冲突与失败回滚 | [存储测试](../crates/store/tests/project_initiative_order.rs)、[HTTP 测试](../crates/control/tests/e2e/project_api/order.rs)、[表格交互测试](../crates/web/spa/src/project/sorting/initiatives.dom.test.jsx) |
| 执行发起、关联及详情切换 | [launcher.dom.test.jsx](../crates/web/spa/src/project/execute/launcher.dom.test.jsx)、[todoDrawer.dom.test.jsx](../crates/web/spa/src/project/tests/todoDrawer.dom.test.jsx) |

改变上述功能规则须明确记录用户决定，并同步修改本规则、实现、测试和相关仓库记忆。实现验收遵守 [规则 01](01-mandatory-tests.md)、[规则 02](02-regression-gate.md)；界面变更同时遵守 [规则 05](05-ui-acceptance.md)。
