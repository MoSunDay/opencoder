# SPA 实时区头行对齐 `streaming` 并支持折叠

一句话：SPA 的 `TurnLiveRegion` 头行由常驻 `⬥ live` 改为与 TUI 对齐的
`> streaming`（展开）/ `▸ streaming`（收起），默认展开、点击头行或
Ctrl/Cmd+L 收起；每个新 running turn 重新展开，折叠只隐藏主体，不丢失
嵌套调用行的开合状态。

## 行为口径

- 头行是按钮：展开态 `> streaming`、收起态 `▸ streaming`，带
  `aria-expanded` 与 `data-turn-live-toggle`；旧标签 `⬥ live` 不再出现。
- 头部使用 dim warn：`app.css` 新增 `--oc-warning`，头行
  `color: var(--oc-warning); opacity: 0.65`。
- 默认展开；点击头行切换折叠。主体用 `display: none` 隐藏而**不卸载**，
  因此已打开的 `ToolContent` 调用行在折叠/展开往返后保持打开。
- 组件内监听 Ctrl/Cmd+L，将实时区置为折叠；`TranscriptView` 原有 Ctrl+L
  的 bubble remount 逻辑不变。
- `TurnLiveRegion` 只保留运行中 turn 的定位，内部 `LiveRegionBody` 以
  running turn 的数组位置为 key：流式 rerender 不重置折叠；新 turn 或
  settle 后重新挂载时恢复默认展开。
- `StreamingSay` 仍是实时区上方的兄弟块，`pendingSay` 的 reducer 注释
  同步为“转录流中渲染、绝不进入实时区”。
- 保持既有面板容器、错误标签与 call-row drill-down；不改变
  `StreamingSay`/`StepBody` 语义，也不把实时区改成无边框。本次不处理
  `⤒ 收起` 对实时区的联动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 默认展开且头行引导当前 step | `the streaming header is expanded by default and leads the running step body` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| 点击头行折叠、主体隐藏但保持挂载 | `clicking the streaming header collapses to ▸ streaming and hides the mounted body` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| 折叠/展开保持已打开的调用行 | `re-expanding keeps a call row that was open before the collapse` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| Ctrl+L 折叠且流式 rerender 不重置 | `Ctrl+L collapses the live region and streaming rerenders keep it collapsed` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| settle 后新 turn 恢复展开 | `settle clears the region and the next running turn starts expanded` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| 流式 Say 在实时区上方、绝不进入实时区 | `the running step body is the region's ONLY content — the streaming Say stays in the flow above it` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| 无 running turn 时无实时区（含旧标签） | `streaming Say rides the transcript flow BELOW the list — never inside the live region` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| 实时区仍定位在消息列表之下且不在 bubble 内 | `running turn streams the last step in the live region below the list, not in a bubble` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |

## 定向验证结果

- `cd crates/web/spa && npm test -- src/stepsBlock.dom.test.jsx src/chat.dom.test.jsx`
  - 2 个测试文件通过，49 个测试通过，0 失败（stepsBlock 36、chat 13）。
- `scripts/build-spa.sh`
  - 通过；提交产物 `crates/web/spa/dist/static/app.js` 与
    `crates/web/spa/dist/static/app.css` 已重建。
- `scripts/check-spa-drift.sh`
  - 通过，输出 `spa dist: no drift`。
