Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# 规则 07：项目模块约定

修改项目、专项、TODO 的数据关系、看板、标签、执行关联、结果读取或进度计算之前，先读本规则。核心职责是：**项目承载整体目标，专项组织某一方向的工作，TODO 记录具体要完成的事情，执行记录保存实际执行过程和结果。**

功能入口见 [项目工作台](../features/project/index.md)，实现索引见 [project 模块](../agents/project/index.md)。涉及大脑调度时同时遵守 [规则 06](06-brain-scheduling-contract.md)，涉及 DAG 执行时同时遵守 [规则 04](04-dag-execution-contract.md)。

## 对象、归属与职责

| 对象 | 职责 | 当前主要信息与状态 |
|------|------|--------------------|
| 项目 | 承载整体目标、说明与总体进度 | 名称、说明、排序；`active` 进行中、`archived` 已归档 |
| 专项 | 组织某一方向的一组工作 | 名称、说明、可选所属项目、排序；`planned` 未开始、`in_progress` 进行中、`done` 已完成 |
| TODO | 跟踪具体任务及交付结果 | 标题、任务说明、可选所属专项、看板列、位置、标签、执行方式和指派历史 |
| 执行记录 | 记录为 TODO 发起或关联的一次执行 | 能力 ID、执行 ID、执行类型、名称和创建时间；状态与结论来自所属执行节点 |

- 层级是 TODO → 可选专项 → 可选项目。一个项目可含多个专项，一个专项可含多个 TODO；每个专项最多归属一个项目，每个 TODO 最多归属一个专项。
- 专项可以独立存在，TODO 可以未归属专项；TODO 通过专项确定所属项目，不直接挂载项目。新建或调整归属时须验证目标存在。
- 项目在代码和 API 中使用 `goal`，专项使用 `initiative`；不能因为内部名称不同再增加一套并列对象。
- 项目、专项和 TODO 负责组织与跟踪工作。任务拆分、保存和指派由用户显式发起，保存任务说明不自动解析能力、创建执行或展开大脑计划。
- 一个 TODO 可以关联多次执行，每次保留独立执行引用。不能将 TODO 等同于某个 Agent 会话、某次 DAG 运行或某次 Brain 激活。
- 项目里程碑入口已移除，存量里程碑下的 TODO 已转为未归属任务；不能重新加入旧里程碑层级。大脑计划内的里程碑层属于执行编排，与专项没有自动的一一映射。

## 从任务到执行的流程

1. 创建或编辑 TODO，保存标题、任务说明、归属、标签和看板状态。
2. 选择能力库中的实际能力 ID，填写任务与命名参数。Agent、Operator、Team、DAG、TODO 工作流及已保存 Brain 计划复用原生执行入口。
3. 通过 `POST /api/project/todos/:id/dispatch` 派发，携带稳定的执行 ID。Fleet 回执固定能力和输入，重试复用原执行，不重复创建任务。
4. TODO 保存执行关联；也可粘贴已有执行 ID。关联接口核验实际执行和准入能力，重复关联幂等。已受理但关联失败时保留执行 ID，可重试关联。
5. 打开执行结论时，通过 `GET /api/executions/:id/result` 向所属节点读取；过程、消息、步骤及完整产物从同一原生执行明细查看。

每次指派保留独立执行引用；新指派不覆盖旧指派。解除关联只移除引用，不取消或删除原执行。

## 看板状态与执行状态

- TODO 看板列固定为 `backlog` 待整理、`todo` 待办、`in_progress` 进行中、`done` 已完成，由用户编辑或拖动维护。
- 执行状态来自原生执行索引。执行结束或取得结果均不能自动将 TODO 标为已完成，也不能作为任务验收通过的依据。
- 项目执行关联只保存 TODO ID、执行 ID、能力 ID、类型、名称和创建时间。不保存结论副本或同步状态，不启动后台结论收集，不提供重新收集接口。
- 查询失败显示实际错误；节点离线返回不可用。界面清除旧的实时结果并提供重试，不能将旧内容当作当前执行结果，也不能因节点离线修改看板状态。
- 原生执行明细由执行 ID 和所属节点定位；项目页面不另建执行状态机。运行中的 Team 可通过原生详情提交引导。
- 旧 `ProjectTodoStatus` 的 `draft/planned/running/done/failed` 属于旧项目执行路径；当前看板、进度和用户编辑以 `board_status` 为准。

## 结果来源与边界

- Agent、Operator 使用节点保存的助手输出正文；Team 使用最终总结；TODO 工作流读取候选结果；Brain 使用调度运行总结。
- DAG 的完整输出保留在步骤明细和产物接口；执行结论入口提供摘要和步骤入口，不把部分步骤结果冒充整次运行结果。
- 实时结论有展示上限；超长或省略内容必须明确提示从原生明细读取完整结果。尚未提供结论、读取失败与执行失败须分别展示。
- 完整结果、消息、步骤和产物始终由所属执行节点持有。历史执行沿用原 Runtime 归属，读取不触发迁移或改写执行。
- 项目和会话使用同一 libsql 实例；schema 33 的关联表只保留执行引用。不得重新引入已移除的结论缓存字段、后台回写流程或可选 SQL 后端。

## 进度计算

- 专项进度＝该专项中 `board_status = done` 的 TODO 数 ÷ 该专项 TODO 总数。
- 项目进度＝所属全部专项中 `board_status = done` 的 TODO 数 ÷ 所属全部专项 TODO 总数；不能简单平均专项百分比。
- 无 TODO 时显示 0%；未归属专项的 TODO 不计入任何专项或项目，独立专项的 TODO 不计入项目。
- 多标签分组造成的重复展示只计同一 TODO 一次；筛选后的可见卡片数量不能替代整体进度的分母。
- 专项自身状态、项目是否归档和完成比例分别维护；进度达到 100% 不自动修改专项状态或归档项目。

例：专项 A 有 1 个 TODO 且已完成，专项 B 有 9 个 TODO 且均未完成，所属项目进度为 10%。

## 标签与页面操作

- 项目模块的独立 Tag 表格统一管理标签，可创建、修改名称和项目或专项归属、删除；每个 Tag 归属一个项目或专项，修改后保留 ID，同一范围内名称唯一。专项继承所属项目的 Tag，同名时使用专项定义。
- 一个 TODO 可选多个当前归属范围内的 Tag；未归属专项的 TODO 没有可选 Tag，不能接受其他范围的标签关联。
- 改变归属、覆盖或删除标签定义后，按名称匹配新的可用标签；有同名定义则接替，没有则清除失效关联。同时修改标签名称和归属时按保存后的名称匹配；未选择标签的 TODO 不自动添加标签。无效标签选择不能留下部分成功的任务或关联修改。
- 按 Tag 分组时，一个 TODO 可展示在多个组内；状态变化须同步到各处展示，保存的仍是同一条 TODO。
- 工作台保留项目、专项、TODO、Tag 四个表格页签；Tag 表格保留项目和专项的各条定义，优先级用于实际标签选择。项目详情展示专项与进度，专项详情展示 TODO 看板，TODO 详情展示任务与指派记录；标签管理统一从 Tag 页签操作。
- 筛选条件在刷新、保存和关闭后重新打开详情时保留。筛选后拖动须保留隐藏卡片的相对顺序；保存失败恢复原看板状态。
- TODO 总表包含所有归属和未归属任务；已有悬空关系在概览中展示为独立专项或未归属 TODO，不能静默丢失。

## 删除与解除归属

- 删除项目时，保留其专项、TODO 和执行记录；专项解除项目归属，项目 Tag 按剩余可用定义重新匹配。
- 含 TODO 的专项不能删除，须先移动 TODO 或解除归属；不能通过删除专项连带删除任务。
- 删除 TODO 时，清理该任务的标签关联、执行关联和旧 Project 运行记录。已关联的原生执行仍由执行系统管理，删除任务或解除关联不作为取消执行命令。

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
| 六类执行关联、能力核验与幂等 | [project_links.rs](../crates/control/tests/e2e/project_links.rs) |
| 实时结果、离线清空与读取重试 | [result.dom.test.jsx](../crates/web/spa/src/project/execute/result.dom.test.jsx)、[results/mod.rs](../crates/control/src/api/executions/results/mod.rs) |
| 看板完成数与项目加权进度 | [overview.rs](../crates/store/src/project/overview.rs) 内联测试、[catalog.test.js](../crates/web/spa/src/project/model/catalog.test.js) |
| 归属编辑、看板移动与筛选保留 | [relations.dom.test.jsx](../crates/web/spa/src/project/views/relations.dom.test.jsx)、[board.test.js](../crates/web/spa/src/project/model/board.test.js)、[project.dom.test.jsx](../crates/web/spa/src/project/project.dom.test.jsx) |
| 执行发起、关联及详情切换 | [launcher.dom.test.jsx](../crates/web/spa/src/project/execute/launcher.dom.test.jsx)、[todoDrawer.dom.test.jsx](../crates/web/spa/src/project/tests/todoDrawer.dom.test.jsx) |

改变上述功能规则须明确记录用户决定，并同步修改本规则、实现、测试和相关仓库记忆。实现验收遵守 [规则 01](01-mandatory-tests.md)、[规则 02](02-regression-gate.md)；界面变更同时遵守 [规则 05](05-ui-acceptance.md)。
