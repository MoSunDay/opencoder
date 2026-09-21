Commit: fc047704e4c583cb9e0c11293b3815916ce1659b

# Web DAG 图连线偶发不显示：声明节点尺寸根治测量门控

DAG 运行图与编辑器画布的依赖连线「有概率不显示、刷新才恢复」。React Flow v12 的边是门控渲染：两端节点都满足 `isNodeInitialized`（handle bounds + 一个宽度）才画，否则静默 `return null`。我们的节点从不声明 `width/height/handles`，初始化完全押在 ResizeObserver 测量时序上；而后台轮询/SSE 每帧 `publish` 全新 snapshot → `useMemo` 重建所有 node 对象 → RF `adoptUserNodes` 把 `measured` 重置、`handleBounds` 置空 → 全部边整体卸载，恢复依赖 RO 回调（后台标签页渲染步进暂停、抽屉动画期间测得 0×0 且无重试路径时，边永久消失）。

四个触发面全部收口，改动仅在 `crates/web/spa`（源码 + dist 重建），服务端零改动：

- **根治（运行图 `dagProjection.js` / 编辑器 `editor/canvasModel.js`）**：`graphFromSpec` 新增内部 `runNodeBox()`、`canvasModel.js` 导出 `editNodeBox()`，节点声明固定卡片盒的 width 与 handles（运行图 176 + 6px、编辑器 240 + 10px，镜像 dagre 常量与 `.dag-node`/`.dag-edit-node` CSS，handle 跨边界故 x 取 `-size/2` / `W-size/2`；**不声明 height**，见下方「复查修订」）。RF 对带 `handles` 的 userNode 每次从声明重建 bounds，snapshot churn 不再把节点打回未初始化，边首帧确定性渲染；RO 之后仍测量并覆盖为 DOM 真值（卡片因 error 文本长高时端点自动吸附）。每次调用返回全新对象（RF 的 `toHandleBounds` 会原地改写 handle 条目）。编辑器 `canvasEditor.jsx` 的 `addStep` 同样带上声明盒——新节点一旦连线不再回到测量门控。
- **边 id 去歧义**：`'e-' + dep + '-' + name` 的连字符与 slug 字符集（`[a-z0-9-]`）冲突，`a→b-c` 与 `a-b→c` 同 id `e-a-b-c` 撞 React key 吞边。三处（`dagProjection.js`、`canvasModel.js`、`canvasEditor.jsx` renameNode）统一改为 `'e-' + dep + '>' + name`（`>` 不可能出现在 slug 中）。
- **编辑器 fitView 竞态（`canvasEditor.jsx`）**：删除挂载后 60ms `setTimeout(fitView)`（与 antd Drawer 300ms 动画、autoLayout 的 50ms 定时器竞争，会 fit 到未测量子集、把边留在视口外），改用 `useNodesInitialized()` 门控的首帧 fit；`autoLayout` 的 refit 改走 `fitEpoch` 计数 effect（setNodes 提交后确定性触发，不再猜 50ms）。
- **连线模式 Handle 干扰（`app.css`）**：`linkMode` 开启时 stage 加 `dag-edit-stage--linkmode` 类，CSS 对 `.react-flow__handle`（含 `connectionindicator`/`connectingfrom`）强制 `pointer-events: none`——第二击不再被卡片边缘 10px Handle 吞掉变成 `onConnectEnd` 的「拖拽落空」静默 re-arm；非连线模式保留 Handle 拖拽连线与 re-arm 语义。
- 删除排查遗留的调试用例 `src/__dbg/dbg.test.jsx`（其 console.log 输出的 `.react-flow__edge` 恒为 0 正是 RO shim 盲区的第一现场）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行图节点声明尺寸/handles | `节点声明固定 width 与左右 handles 且不声明 height（边首帧即渲染、卡片高度归 RO）` | `crates/web/spa/src/dagProjection.test.js` |
| 运行图连字符命名边 id 唯一 | `连字符命名不撞边 id（a→b-c 与 a-b→c 不再折叠成同一条边）` | `crates/web/spa/src/dagProjection.test.js` |
| 编辑器节点声明尺寸/handles | `节点声明固定 width 与左右 handles 且不声明 height（边首帧即渲染、卡片高度归 RO）` | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 编辑器连字符命名边 id 唯一 | `连字符命名不撞边 id（a→b-c 与 a-b→c 不再折叠成同一条边）` | `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 运行图 DOM 边可见（jsdom 无 RO 也渲染） | `shows final states immediately...`（内嵌 `.react-flow__edge` 数量断言） | `crates/web/spa/src/dag/graph.dom.test.jsx` |
| 编辑器 DOM 边可见 | `画布模式默认渲染 spec 步骤节点与依赖连线` | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 连线模式落边后 DOM 边数 | `连线模式点击两个步骤建立依赖并保存`（内嵌边数断言） | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 拖拽/click 连线路径 addEdge 显式 '>' id 且唯一 | `连线模式点击两个步骤建立依赖并保存`（内嵌 `data-id="e-review>step"` 与 Set 去重断言） | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 节点不声明 height（防内联高度回归） | `节点声明固定 width 与左右 handles 且不声明 height（边首帧即渲染、卡片高度归 RO）` | `crates/web/spa/src/dagProjection.test.js` / `crates/web/spa/src/dag/editor/canvasModel.test.js` |
| 改名后边 id 随名更新且唯一 | `改名含连字符的步骤后边 id 仍唯一且随名更新` | `crates/web/spa/src/dag/editor/editor.dom.test.jsx` |
| 既有边 id 断言同步新格式 | `循环依赖的两条连线都被保留` 等 | `crates/web/spa/src/dag/editor/canvasModel.test.js` |

- SPA 回归：`npm test`（crates/web/spa）→ 120 文件 / **892 passed / 0 failed**（基线 888：+5 新用例、-1 调试用例）；定向 `npx vitest run src/dagProjection.test.js src/dag/editor src/dag/graph.dom.test.jsx` → 62 passed。
- DOM 边断言在 jsdom（RO shim 永不回调）下通过，证明边渲染不再依赖测量时序，该缺陷自此有回归保护。
- `npm run build` 重建 `dist/static/app.js` / `app.css`（仓库惯例 dist 随源码提交），抽查产物含 `"e-"+…+">"`、handles 声明与 `--linkmode` 规则。
- 本条画布修复不改服务端逻辑；同批发布还包含并发调度和磁盘准入调整，整体 Rust/SPA 验证见 `storage-admission-threshold.md`。

## 复查修订（同日，实施后复查发现的三点收紧）

1. **取消声明 `height`**：`getNodeInlineStyleDimensions()` 在 handleBounds 已定义时仍会返回 `node.height`，声明盒带 `height` 会给节点 wrapper 永久内联 `height:52px/72px`；而 `.dag-node`/`.dag-edit-node` 无 `position:relative`（Handle 的 `top:50%` 以 wrapper 为基准），error 文本撑高卡片时 handle 停在 52px 中点错位、`measured.height` 被钳导致 fitView 低估——改变了线上 auto-height 行为。`runNodeBox()`/`editNodeBox()` 改为仅声明 `width + handles`：`isNodeInitialized` 只要求宽度，边渲染门控不受影响；height 回退 `undefined` 后 wrapper 回到 auto-height、RO 报真值（与修复前一致）。单测同步加护栏 `expect(n.height).toBeUndefined()`。
2. **`onConnect` 补显式边 id**：拖拽/连线模式走 `addEdge({...params})` 未传 id，默认 `getEdgeId` 生成 `xy-edge__{source}-{target}`，`a→b-c` 与 `a-b→c` 的撞 key 问题在该路径仍存在。`onConnect` 显式 `id: 'e-' + source + '>' + target`，与三处静态建边同格式（`isEdgeBase` 的 `'id' in element` 检查会保留显式 id）。
3. **mount-fit 加 once-guard**：store 的 `nodesInitialized` 要求 measured 宽高齐备，`addStep` 新节点无 measured → 标志翻 false→RO 测完→true，`[nodesReady]` effect 会再次 fitView，加步骤时视口意外 refit。加 `fittedRef` once-guard，首帧 fit 仅每次挂载首次 ready 触发一次；`fitEpoch`（autoLayout）refit 通道不变。

- SPA 回归（修订后）：`npx vitest run` → 120 文件 / **892 passed / 0 failed**；`npm run build` 重建 dist，抽查产物含 `id:"e-"+…+">"`、once-guard（`….current=!0,…({padding:.18,duration:200})`）且无节点声明高度（dist 内残留 `height:52/72` 仅为 dagre `g.setNode` 布局常量）。
