Commit: c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6（开发基线；多轮工作树成果随本提交落地）

# DAG 工作流可视化画布编辑器

## 问题与行为

工作流定义此前只有 JSON textarea（`dag/defEditor.jsx`），编排依赖靠手写 `depends_on` 数组，无任何可视化反馈。现在定义抽屉升级为「画布（默认）/ JSON」双模式：画布复用树内 `@xyflow/react` v12 + `@dagrejs/dagre`（运行态图已用），零新依赖，后端与 spec/protocol LOCKED 契约零改动。

画布核心交互：

- **节点面板**：Agent / Python 步骤卡片，点击或 HTML5 拖入画布，新步骤自动生成唯一 slug。
- **节点卡片**：kind 图标 + 步骤名 + kind 标签 + 依赖摘要；校验异常红点（`validateSpec` 实时镜像）。
- **连线即依赖**：左入右出 Handle，smoothstep 箭头连线；`canConnect` 即时守卫拒绝自连 / 重复边 / 成环（`message.warn`，不落边），拖拽过程中 `isValidConnection` 同步阻止非法目标。
- **属性面板**：点选节点右侧滑出 —— 步骤名（slug 校验 + 重命名同步节点/边/位置）、kind 切换（重置载荷）、agent 的 prompt/agent/model、python 的 code/sandbox、timeout_secs；未选中时显示画布设置（spec.name/description）。
- **工具条**：自动布局（dagre LR 重排）、适应画布、校验问题徽标列表。
- **快捷键**：Delete/Backspace 删除选中节点或边（输入框内按键不误删）；删除节点同步清空两端依赖。
- **自适应**：mount 自动布局 + fitView，ResizeObserver 重适配；窄屏抽屉面板纵排。
- **JSON ↔ 画布无损互转**：`canvasModel.js` 纯函数层保证 spec ⇄ {nodes, edges} 往返保真 —— 未知依赖（ghost deps）与环依赖在往返中原样保留交给校验裁决；位置仅存编辑器会话态（按步骤名 key），重开自动布局恢复，不扩 LOCKED 领域。

`DefEditor` 对外接口（open/def/saving/onClose/onSave）与 `defsTab.jsx` 调用方完全不变；服务端 400 问题列表照旧渲染，JSON 模式未解析成功时阻止切回画布。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| spec ⇄ 画布互转 roundtrip（含 description/timeout/sandbox/agent/model 字段、步序稳定） | `roundtrips a representative spec losslessly` 等 | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| ghost 依赖保留、自依赖剔除、重复依赖去重、环边保留 | 同上 describe | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 连线守卫（自连/重复/成环拒绝、合法为 null、三节点路径环） | `canConnect rejects ...` | 同上 |
| 重命名校验（slug 规则 ≤64 字符、重名）、唯一 slug 生成、kind 切换重置载荷 | 同上 describe | 同上 |
| 问题索引（steps[N] → 节点）与 spec 级问题分离 | `specProblemIndex ...` | 同上 |
| 画布默认渲染、面板添加 Python 步骤并保存 payload | `editor.dom.test.jsx` 前 2 例 | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 选中节点属性面板编辑 code 并保存 payload | 第 3 例 | 同上 |
| JSON ↔ 画布模式往返无损（改名 etl→etl2） | 第 4 例 | 同上 |
| JSON 解析失败阻止切换并在保存时报错 | 第 5 例 | 同上 |
| 既有 JSON 模式校验问题 / 服务端 400 问题列表回归 | `DefEditor` describe（补 JSON 模式切换） | `crates/web/spa/src/dag/dag.dom.test.jsx` |

验收结果：

- SPA：`npm test` → 51 个文件、449 个测试通过（新增 20 个）；`scripts/build-spa.sh` 重建 dist（单 bundle、固定文件名契约不变），`scripts/check-spa-drift.sh` → no drift。
- Rust：`cargo test -p opencoder-web --locked` → 252 passed / 1 failed；唯一失败 `goal_milestone_todo_crud_contract` 为工作区既有未完成 project-store 改动（`executor_kind` 列）所致，已用 stash 复证与本特性无关（移除本特性改动后同样失败）。html.rs 嵌入白名单 6 项契约测试全部通过。
