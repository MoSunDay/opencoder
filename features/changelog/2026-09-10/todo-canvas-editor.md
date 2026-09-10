Commit: (working-tree, 基于 e321f107)

# SPA：TODO 模板编辑器「画布」可视化模式（表单/画布/JSON 三态）

SPA-only 改动，服务端零改动：`PUT /api/todo/templates/:name/:version/context.json`
本就有 validate_spec 400 兜底，本次不改任何 API 与库表。

## 背景

原状：`todoEditor.jsx` 只有 表单 + JSON 两态。表单模式只覆盖高频字段，
`acceptance.required_tool_calls` 完全没有编辑入口（只能切 JSON 手改）；
依赖关系靠多选下拉想象，无可视化。

## 方案

新增 `crates/web/spa/src/todo/editor/`，架构对齐 `src/dag/editor/`：
spec 是唯一事实来源、画布坐标只进会话 positions 不入库、
结构变更经 onSpecChange 上抛重建。

- `specValidate.js`（213 行，纯函数）— 逐条镜像
  `crates/todos/src/domain.rs::validate_spec`；reject_cycles 同为迭代三色 DFS
  （2000 节点线性链不爆栈）。返回结构化 `[{path,message}]`：todo 级问题
  `path='todos[<id>]'` 供画布节点红点定位，无法定位 id 的回落 `workflow`；
  未知 agent 放行交服务端裁决。
- `canvasModel.js`（255 行，纯函数）— spec↔画布转换；ghost 依赖（depends_on 中
  未连边的引用）保留、去重、保持原顺序并标错，不静默丢弃；`renameTodo` 同步
  节点 id、边与全部 depends_on 引用（含 ghost）；`canConnect` 判自连/重复/成环；
  `uniqueSlug`/`newTodo` 生成新节点唯一 id。
- `canvasLayout.js`（52 行）— dagre 布局。
- 视图层 `canvasEditor.jsx` / `todoNode.jsx` / `canvasToolbar.jsx` /
  `todoInspector.jsx` / `requiredCallsEditor.jsx` — 后者补齐 required_tool_calls
  的添加/命名/`arguments_contains` JSON 编辑（非法 JSON 行内报错不上抛）；
  节点复用 `.dag-edit-*` 样式类。

`todoEditor.jsx` 扩为 表单/画布/JSON 三态外壳（Segmented），切换时序镜像
`dag/defEditor.jsx`：spec 是唯一草稿事实来源，离开表单容忍半填先并入 spec，
离开 JSON 解析失败停留原模式；画布坐标是会话 state（positions + canvasKey
重挂），不入 spec 不入库。`src/app.css` 追加 `.todo-edit-*` 最小样式。

## 取舍

- 客户端校验是**建议性镜像**（提前红点改善编辑体验），服务端 validate_spec
  权威；未知 agent 故意放行，因为 SPA 不持有 agent 全集。
- ghost 依赖保留并标错而非静默删除——删边只解除可视化关系，不丢 spec 里的
  引用信息。
- 表单模式仍不展开 required_tool_calls（低频）：`formToSpec` 按 todo id 自
  original 透传，编辑入口在画布 Inspector 与 JSON 模式。

## 测试覆盖

| 功能 | 测试 | 文件 |
|---|---|---|
| 逐条校验规则镜像（schema_version/非空/id 重复/不安全 id/max_attempts/depends_on/required_tool_calls 形状/agent） | `validateSpec` 30 例，如 `id / name / objective 非空检查（trim 后）`、`id "a/b" 不安全（/ \ .. 空字节）`、`max_attempts %p 必须为正整数 0`、`acceptance 缺失 / criteria 非字符串 / 空白 都算空`、`required_tool_calls arguments_contains 是数组 → 条目非法`、`agent 空 / workflow 拒绝，未知 agent 放行` | `crates/web/spa/src/todo/editor/specValidate.test.js` |
| JSON 草稿解析 | `parseSpecDraft` 2 例（`空文本 / 坏 JSON / 非对象给出中文错误`） | 同上 |
| 环检测（含长链不爆栈）与问题挂载路径 | `findCycle` 4 例（`2000 节点线性链不成环也不爆栈（迭代 DFS）`、`自依赖也算环` 等）+ `环问题挂到 todos[id] 路径`、`无法定位 id（空白/缺失）的 todo 问题挂 workflow` | 同上 |
| spec↔画布 round-trip | `specToCanvas → canvasToSpec 无损还原`、`depends_on 由入边重建且顺序 = source 节点序`、`spec 级字段透传，schema_version 缺省 1` | `crates/web/spa/src/todo/editor/canvasModel.test.js` |
| specToCanvas 边生成与防御 | `depends_on 生成边（id 为 e-<dep>-<id>）`、`自依赖 / 未知依赖 / 非字符串依赖不生成边，重复引用去重`、`环边保留（画布要渲染并标红）`、`无字符串 id 的 todo 不生成节点`、`节点按 todos 顺序生成并携带私有 todo 拷贝` | 同上 |
| ghost deps 保留 | `ghost 依赖保留、去重且保持原顺序（不静默丢弃）`、`无边时画布内 id 的依赖随边消失，ghost 依赖仍在` | 同上 |
| renameTodo 同步 | `成功重命名同步节点 id、data.todo.id、边与 depends_on 引用`、`depends_on 中未连边的引用（ghost 引用）也同步`、`空白 / 重复 id 拒绝，候选 id 会 trim`、`不修改输入画布（不可变风格）` | 同上 |
| canConnect 判环 | `自连 / 重复 / 成环分别给出中文理由，其余可连` | 同上 |
| 问题索引分组 | `todos[id] 条目按 id 分组收集 message`、`workflow 级与无法定位的条目进入 specLevelProblems` | 同上 |
| 新节点 id 生成 | `uniqueSlug 空闲返回原名，冲突时递增 -2 / -3`、`newTodo 生成编辑器默认值与唯一 id` | 同上 |
| 画布 DOM：渲染/加节点/改 title | `渲染 spec 的 TODO 节点与左侧面板`、`面板点击添加 TODO 并上抛三节点 spec`、`选中节点后属性面板编辑标题并上抛`、`未选中时展示 spec 基础信息表单` | `crates/web/spa/src/todo/editor/editor.dom.test.jsx` |
| required_tool_calls 编辑（非法 JSON 不上抛） | `required_tool_calls：添加/填名/合法 JSON 上抛，非法 JSON 行内报错不上抛` | 同上 |
| Inspector 重命名与错误渲染 | `id 失焦时上抛 onRename（重名校验交给父层）`、`回车同样触发 onRename，未修改则不触发`、`problemList 非空时渲染校验错误` | 同上 |
| 三态外壳（`todoEditor.jsx` switchMode） | `加载后落在表单模式并回填 spec 高频字段`、`表单改目标后切「JSON 源码」：新目标并入且 metadata/required_tool_calls 透传`、`JSON 非法时切回表单被阻止：停留 JSON 模式并提示解析失败`、`JSON 改名后切回表单：解析后的 spec 回灌表单`、`画布模式渲染 spec 数量的节点，切回表单编辑不丢`、`表单模式保存：合并 spec PUT 到 context.json，env 未变不重发` | `crates/web/spa/src/todoEditor.dom.test.jsx` |

## 回归

- `cd crates/web/spa && npx vitest run src/todo` → 79/79（6 文件，新增
  `todoEditor.dom.test.jsx` 6 例）。
- 全量 `npx vitest run`（无需排除）→ 647/647（77 文件；含新增三态外壳 6 例，
  toolsDrawer 当次亦全绿）。
- `scripts/build-spa.sh` 已重建 `crates/web/spa/dist`；
  `scripts/check-spa-drift.sh` → no drift (build 1/3)。
- Rust 门禁（本次零 .rs 改动，dist 为唯一 Rust 触点）：`opencode-todos` 90/90、
  `opencode-web`（内嵌重建后 dist）全量 63 个测试套件 0 失败、todo 表面集成测试
  （web_todo_templates/web_todo_envs/web_todo_runs）12/12（三态测试补齐当日复核：todos
  exit 0 / 90 通过、web exit 0 / 63 套件 305 通过 0 失败、drift no drift）；另跑
  `cargo test --workspace --locked --no-fail-fast` 作全量基线，因并行进行中工作的
  机器高负载（load 100+）在 55 分钟超时截断，已完成部分 339 个套件中仅 2 个失败
  且均位于本次未触碰的 crate（`opencode-control` e2e 1 例 / `opencode-node`
  runner_happy），与本次改动无关联。空闲窗口（load<16）补跑同一命令闭合：379 个
  套件 5117 通过，仅 3 个二进制受并行 agent 共享 target/端口争用影响（2 个二进制
  被并发构建删除而未执行、`web_project_runs` 3 例轮询超时），四者同树单独重跑
  全部通过（2/2、6/6、4/4）——workspace 无未解失败，rules/02 闭合。

## 相关语义

- [TODO 工作流](../../../features/todos/index.md)
- [todos 模块](../../../agents/todos/index.md)（validate_spec 权威端）
- [web 模块](../../../agents/web/index.md)
