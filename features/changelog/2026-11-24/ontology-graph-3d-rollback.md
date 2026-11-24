# Ontology 图谱回滚到二维 @ant-design/graphs / @antv/g6

一句话：完整撤销 2026-10-22 的三维图谱及观测直达链接，恢复上一版二维
`@ant-design/graphs` / `@antv/g6` 实现，并移除三维依赖、验收脚本、文档与深度链接。

## 行为与实现

- 图谱恢复为 `FlowDirectionGraph` + G6 二维有向图：`GraphCanvas.tsx`、
  `GraphPage.tsx` 回到二维实现，`canvas/` 重新负责节点渲染、关系边、完整关系
  标签、缩放/平移/视口运行时，`graphLifecycle.ts` 串行化异步渲染与销毁；列表、
  投影与 30 节点展示预算保持不变。
- 删除 `canvas3d/` 整个目录，以及 3D 专属的 `graphLifecycle.test.tsx`、
  `relationshipLabel.test.tsx`；恢复二维的 `canvas/GraphNode.tsx`、
  `RelationshipEdge.ts`、`relationshipLabel.ts`、`useCanvasRuntime.ts`、
  `graphLifecycle.ts`、`graphLifecycle.test.ts`、`relationshipLabel.test.ts` 与
  `pageFixture.tsx`。
- 删除 `ontology/navigation/observationLink.ts` 及其测试和
  `graph/session/linkedObservation.ts`；`main.jsx`、`fleet/navigation/links.js`、
  `fleet/navigation/location.js` 回到二维版本，不再识别 `#ontology_aspect` 等
  观测参数。测试适配 `test/appShell.jsx`、`shell/headerContract.dom.test.jsx`
  恢复为 mock `@ant-design/graphs` 的 `FlowDirectionGraph`。
- `package.json` / `package-lock.json` 移除 `react-force-graph-3d`、`three`、
  `@types/three`，恢复 `@ant-design/graphs` `^2.1.1` 与 `@antv/g6` `^5.1.1`。
- 删除三维验收 `scripts/acceptance/ontology/graph.mjs`、`graphVisual.mjs`、
  `graph.py`；`experience.mjs` 恢复为二维图谱交互检查；`catalog.mjs`、
  `catalog.py`、`entities.mjs` 保留。
- 文档同步二维实现：`agents/ontology/index.md`、`agents/frontend/web/ontology.md`、
  `features/platform/ontology/index.md`；2026-10-22 两篇三维变更记录加入“已回滚”
  提示并指向本记录。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 延迟销毁回调与实时事件绑定 | `ignores a delayed destroyed callback and binds live handlers once` | `crates/web/spa/src/ontology/pages/graph/__tests__/GraphCanvas.test.tsx` |
| 缩放与自动取景动作 | `provides explicit zoom and overview actions` | `crates/web/spa/src/ontology/pages/graph/__tests__/GraphCanvas.test.tsx` |
| 选择可见的观测中心 | `chooses a visible center` | `crates/web/spa/src/ontology/pages/graph/__tests__/GraphCanvas.test.tsx` |
| 密集关系缩放仍保留完整标签 | `keeps relationship labels when zooming a dense graph` | `crates/web/spa/src/ontology/pages/graph/__tests__/GraphCanvas.test.tsx` |
| 异步图谱生命周期 | `async graph lifecycle` | `crates/web/spa/src/ontology/pages/graph/__tests__/graphLifecycle.test.ts` |
| 完整关系标签 | `complete relationship labels` | `crates/web/spa/src/ontology/pages/graph/__tests__/relationshipLabel.test.ts` |
| 移除观测链接后的导航与位置解析 | 既有 `encodes execution IDs and keeps execution links ahead of stored navigation`、`follows ID-only links, hash changes and browser history without relying on the list` | `crates/web/spa/src/fleet/navigation/links.dom.test.jsx` |

## 验证

- `npm ci` 恢复二维依赖；定向 Vitest 4 个文件、24 个用例通过：
  `GraphCanvas.test.tsx`、`GraphPageAspects.test.tsx`、
  `graphLifecycle.test.ts`、`relationshipLabel.test.ts`。
- 导航链接定向回归 `src/fleet/navigation/links.dom.test.jsx` 9 个用例通过。
- 未运行全量 SPA 测试、浏览器验收与 Rust 测试；SPA 构建与 dist 产物核验见本次父流程。
- 收尾：dist 随品牌导航收尾提交重建（`spa dist: no drift`）；`fleet/navigation` 两个自读取请求传入 `{ signal }` 起失效的既有断言已对齐（`d1d017483`），该目录 4 个文件 20/20。
