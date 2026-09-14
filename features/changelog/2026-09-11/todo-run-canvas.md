Commit: working-tree

# TODO 运行调度画布接入 Web 运行详情

## Context

TODO 框架的运行态此前在 Web 上只有两张表（TODO 项 + 事件流水）：父 Workflow Session（"大脑"）怎么调度、哪个 TODO 在验收、哪个被打回，全靠肉眼读文本事件。模板编辑侧已有画布（`todo/editor/`），运行侧没有对应的图形投影。

## Change Summary

`todoRunsPanel` 的运行详情新增「调度画布」：`runProjection.js`（纯函数）把 Store items 快照与 SSE `todo_*` 事件帧折叠成每 TODO 视图状态，投影到 spec 依赖图上（只读 React Flow，dagre 自动布局复用编辑器 `canvasLayout.js`）；`runCanvas.jsx` 渲染节点卡（状态 Tag / 尝试次数 / 依赖）并联动 Inspector（验收标准、必需工具调用、当前会话、最近错误）。SSE 帧直通详情页：`todo_*` 实时折叠，`workflow_rewound/suspended/resumed` 触发静默重拉纠正投影；工作流头部展示 spec 目标与进度统计（已通过/执行中/失败/待执行）。TODO 项表与事件流改为双栏布局。

## Impact Surface

- `crates/web/spa/src/todo/runProjection.js`（新增，纯投影）、`runProjection.test.js`（新增）
- `crates/web/spa/src/todo/runCanvas.jsx`（新增）、`runCanvas.dom.test.jsx`（新增）
- `crates/web/spa/src/todoRunsPanel.jsx`（运行详情接入画布 + 双栏）
- `crates/web/spa/dist/*`（重建产物，cargo 内嵌 SPA 随之更新）
- 服务端与 todos 运行时零改动（数据全部来自既有 `/api/todo/workflows*` 与 SSE）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| items/SSE 折叠、幂等回放、执行失败语义 | `foldTodoEvents` 系列 | `crates/web/spa/src/todo/runProjection.test.js` |
| 依赖图投影（self/unknown/dup 边丢弃）与进度统计 | `runGraph`、`runProgress` 系列 | `crates/web/spa/src/todo/runProjection.test.js` |
| 状态 → 节点色板映射 | `todoRunClass` | `crates/web/spa/src/todo/runProjection.test.js` |
| 画布挂载、运行状态/尝试/依赖可见、点选回调、空态 | `TodoRunCanvas` | `crates/web/spa/src/todo/runCanvas.dom.test.jsx` |
| Inspector 验收/错误/工具调用呈现 | `TodoRunInspector` | `crates/web/spa/src/todo/runCanvas.dom.test.jsx` |

- SPA 全量回归：`npm test`（vitest）→ 83 files / 674 passed / 0 failed
- 既有运行面板/编辑器回归：`todoRunsPanel.dom.test.jsx`、`todoEditor.dom.test.jsx`、`todo/editor/*` → 79 passed
- SPA 构建：`npm run build` + `scripts/check-spa-drift.sh` → no drift
- Rust 零改动；`dist` 为编译期内嵌产物，无需 cargo 回归
