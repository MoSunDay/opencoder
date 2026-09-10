# SPA：Env 工具跟随 env（右滑 TOOLS 抽屉）+ 大脑能力编辑器「类型 → 名称」级联

两个需求均为 SPA-only 改动，后端 API 与库表零变更（所需端点在 web/control 两面均已存在）。

## 1. Env 管理 — 工具跟 ENV 走

原状：ENV 表格下方挂一个**全局**「工具目录」（ToolsCatalog），工具绑定藏在编辑抽屉的
多选框里——工具不跟 ENV 走，且「选了可导入项会被 400」的坑常驻提示。

改动：

- 新增 `src/envs/toolsDrawer.jsx`（202 行）：点 env 行（或「工具」列 Tag）右滑打开
  `EnvToolsDrawer`（placement=right / width=720 / destroyOnHidden）。三段内容：
  ① 头部 env 名称/描述 + 工具数/变量数统计；② 「已绑定工具」表（等宽 ref + 行内移除）；
  ③ 「添加工具」多选（选项 = share 中未绑定 ref）+「可导入工具」表（agent/version/tool + 导入）。
  绑定变更走 `PUT /api/todo/envs/:name` 的**部分合并**语义（只发 `{tools}`，description/env_vars
  由服务端保留），成功后 `onChanged` 让父面板 silent 刷新（工具数列同步）；导入走
  `POST /api/todo/tools/import`，成功后只静默重拉目录（导入不改绑定）。行级互斥用单个
  `busy` 串（`remove:`/`add`/`import:`），PUT 400（工具引用无法解析）经 onNotice 透出。
- 新增 `src/envs/envModel.js`（23 行，纯函数）：`envFromContext`（自 envsPanel 迁出）+
  `shareTools`/`importableTools`。
- `src/envsPanel.jsx`（399 → 314 行）：删除全局 ToolsCatalog 与编辑抽屉的 tools 多选
  （编辑抽屉瘦身为 描述 + env_vars，保存只发这两个键）；表格 `onRow` 点击打开工具抽屉，
  「操作」列 `stopPropagation` 防误触（brainPanel 先例），「工具」列 Tag 为可点入口。
  原「选可导入项会被 400」提示随多选下线一并删除。

## 2. 大脑能力编辑器 — 类型 → 名称 级联

原状：`能力类型`（自由文本）与「执行目标」区块的 `执行类型`+`执行目标`（手输）语义重叠。

改动：

- 新增 `src/brain/targetOptions.js`（49 行，纯函数）：`TARGET_ENDPOINTS`（agent→/api/agents、
  team→/api/teams、dag→/api/dag/defs、todos→/api/todo/templates）+ 四类响应归一化
  （dag 裸数组/`{defs}` 两形态、todos `${name}/${version}` 逐版本展开、其余取 name，
  全程防御性过滤）+ `fetchTargetOptions(apiGet, kind)`（未知 kind 不发请求返回 []）。
- `src/brain/capabilityEditor.jsx`（98 → 121 行）：删 `能力类型` Input、`执行目标` 分割线与
  自由文本；表单变为 ① `执行类型`（Select，置顶）② `名称`（Select showSearch，**级联**：
  `Form.useWatch('target_kind')` 驱动选项加载，切换类型清空名称）③ 其余字段不动。
  编辑态已绑定但资源已不存在的 target 由 watch 兜底保留显示。两步保存流
  （内容 PUT + target PUT）与失败文案二分逐字未动。
- `src/brain/model.js`：`capabilityBody` 派生 `capability_type: values.target_kind`
  （后端 validate 要求非空；embed 文本「类型: agent」语义成立，免后端改动）；
  `capabilityForm` 不再产出 capability_type。
- `src/brainPanel.jsx`：列表列「能力类型」→「执行类型」，render `KIND_LABELS[value] || value`
  （存量自由文本分类仍原样展示）。

## 取舍

- 旧能力数据的 `capability_type`（自由文本分类）在下次编辑保存时被覆盖为执行类型值——
  这是「能力类型改成执行类型」的直接语义，计划内行为；存量记录只读展示不受影响。
- 名称选项即目标来源：agent→`.agents[].name`、team→`.teams[].name`、dag→`def.id`、
  todos→`${name}/${version}`。

## 测试覆盖

| 功能 | 测试 | 文件 |
|---|---|---|
| 工具抽屉打开/绑定清单/计数 | `shows bound tools, the importable catalog and counts on row click` | `crates/web/spa/src/envs/toolsDrawer.dom.test.jsx` |
| 移除 → PUT body 不含该 ref | `removes a bound tool via PUT with the emptied tools list` | 同上 |
| 添加 share 未绑定工具 → PUT 去重全量 tools | `adds a pending share tool via PUT with the deduped full tools list` | 同上 |
| 导入命中 POST /api/todo/tools/import | `imports an importable tool via POST and silently refreshes the catalog` | 同上 |
| PUT 400 经 onNotice 透出且绑定不丢 | `surfaces a PUT failure via onNotice and keeps the binding` | 同上 |
| env 面板：全局目录下线、行点击/编辑入口互不误触、编辑保存 body 不含 tools | `EnvsPanel` 7 例 | `crates/web/spa/src/envsPanel.dom.test.jsx` |
| 四类端点归一化（dag 两形态/todos 拼接/防御过滤/未知 kind/端点命中） | 8 例 | `crates/web/spa/src/brain/targetOptions.test.js` |
| 级联加载（/api/agents→切 DAG→/api/dag/defs 且清空名称）、新 POST/PUT body `capability_type==='agent'`、无旧标签守卫、列头「执行类型」+ KIND_LABELS/自由文本回退断言、两步保存守卫 | 9 例 | `crates/web/spa/src/brainPanel.dom.test.jsx` |

门禁：`npm test` 本迭代独立树 577/577（73 文件；混合工作树全量 641/641，76 文件）；`npm run build` + `scripts/check-spa-drift.sh` no drift；
`scripts/acceptance/spa_responsive.js` 27 项测量 0 越线（390×844，含 Env 管理页）；
后端零改动，`cargo test -p opencoder-web -p opencoder-control` 兜底确认无契约破坏。
