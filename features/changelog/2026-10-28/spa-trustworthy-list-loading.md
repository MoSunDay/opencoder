# SPA 可信列表加载：消灭撒谎空态 + 可见性感知轮询

一句话：把 `ui/tableLoading.js` 自我声明的「唯一约定」补齐到剩余三个高频入口（调度触发历史、能力库、项目 Overview 三页签），新增 `ui/usePoll.js` 统一轮询（`document.hidden` 挂起、回前台补拉、失败保留旧数据），执行列表补「仅显示最近 50 条」上界与三态空态——首屏数据在路上时，页面不再对用户说「还没有」。

## P0 撒谎空态（loading 中不渲染「暂无…」）

- `schedule/history.jsx`：触发历史 Drawer 原为裸 `loading={loading}` + `dataSource={data || []}`，首拉期间一边转圈一边显示「暂无触发记录」。接入 `tableLoading/tableRows`。
- `brainPanel.jsx`：`rows` 初始改 `null`（未拉到 = 不知道，而非空列表），首拉/搜索期间 `dataSource` 交回 `undefined`；「没有匹配的能力」的搜索空态语义保留（`hits !== null`）。
- `project/project.jsx` + `views/projectTable.jsx`：`ProjectTable` 显式接 `loading`（默认 `false`，其余调用方行为不变）；项目/子项/TODO 三个页签透传；移除整页 `<Spin spinning={loading}>` 包裹（避免双重遮罩，页签在加载期间可切换），表格各自用带 delay 的遮罩表达进度。

## P1 `ui/usePoll.js`（新文件，纯函数 hook）

- 契约：挂载即 `first`（响亮首拉，遮罩语义由调用方定），之后按 `intervalMs` 静默 `poll`；`document.hidden` 时挂起定时器，`visibilitychange` 回前台立即补拉一次并重启完整间隔；回调身份变化不重置节奏（ref 持最新值），`deps` 变化重跑 `first` 并重置节奏。
- 首批接入 4 处，间隔与静默语义均不变：
  - `fleet/useNodes.js`（5s）：出错不再 `setNodes([])`——节点清空会让 schedule「指定节点」列、chat、brain 启动、节点总览四个消费方一起闪没，现在失败保留旧节点、只更新错误文案；
  - `fleet/executions.jsx`（3s，`deps: [filter]` 保持筛选变更即响亮重拉）；
  - `schedule/panel.jsx`（5s）；
  - `project/useOverview.js`（8s，静默 refresh 语义不变）。
- 其余 13 处轮询（detail、questionModal、brain useRun、review 等）留作后续批次。

## P2 执行列表知情权（不动 API）

- `fleet/executions.jsx`：`executionPagePath` 默认 `limit=50` 且丢弃 `next_cursor`，用户以为看到全部历史。补 `pagination.showTotal`「仅显示最近 50 条」（antd 空列表不渲染分页，有数据时才出现，恰好只在需要时告知上界）与三态 `locale.emptyText`：读取失败 / 真·空（`暂无执行`）/ 拉取中（由 `tableRows` 抑制）。上界数字与 `model.js` 的 `EXECUTION_PAGE_LIMIT` 常量同源（`executionPagePath` 默认值同改引用），改 limit 时文案不再各自为政。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 首拉 + 间隔轮询 | `mount runs first once, then polls every intervalMs while visible`（src/ui/usePoll.test.js） |
| 后台标签页挂起轮询 | `suspends polling while the tab is hidden`（同上） |
| 回前台立即补拉并重启间隔 | `fires one immediate catch-up poll on returning to foreground, then restarts the interval`（同上） |
| 隐藏状态下挂载：只首拉、不布防 | `runs first when mounting hidden but does not arm the timer`（同上） |
| 卸载清理定时器与监听器 | `clears the timer and removes the listener on unmount`（同上） |
| 轮询失败不打断节奏（保留旧数据） | `keeps polling on schedule after a failing poll that keeps old data`（同上） |
| deps 变化重跑 first 并重置节奏 | `re-runs first and resets the rhythm when deps change`（同上） |
| 节点轮询失败保留旧列表 | `keeps the last node list when the 5s poll fails, instead of flashing it away`（src/fleet/useNodes.test.js） |
| 触发历史首拉不显示「暂无触发记录」 | `does not claim 暂无触发记录 while the first fetch is still in flight`（src/schedule/history.dom.test.jsx） |
| 能力库首拉不显示「暂无能力」 | `does not claim 暂无能力 while the first fetch is still in flight`（src/brainPanel.dom.test.jsx） |
| 项目页首拉不显示「还没有项目」 | `does not claim 还没有项目 while the first overview fetch is still in flight`（src/project/project.dom.test.jsx） |
| 执行列表失败/真空/上界三态 | `separates a failed read from a true empty list and states the 50-entry cap`（src/fleet/fleet.dom.test.jsx） |
| 既有约定回归（遮罩 delay、静默轮询不锁行内按钮、假空态抑制） | `src/ui/tableLoading.dom.test.jsx`（5 项）、`src/ui/tableLoading.test.js`（5 项）原样通过 |

## 验证范围

只运行本次改动及直接影响链路的定向检查（规则 02），未运行全量测试。

| 实际命令（crates/web/spa 下） | 结果 |
| --- | --- |
| `npx vitest run src/ui src/schedule src/brainPanel.dom.test.jsx src/brainPanel.pagination.dom.test.jsx src/project src/fleet/fleet.dom.test.jsx src/fleet/executionControls.dom.test.jsx src/fleet/navigation/links.dom.test.jsx src/dag/dag.dom.test.jsx` | 48 文件 / 283 项全部通过（含新增三态空态用例；覆盖 tableLoading 约定、三处首拉不撒谎、项目页全链路、useNodes 的 fleet/dag 消费方） |
| `npx vitest run src/fleet/fleet.dom.test.jsx src/fleet/useNodes.test.js` | 2 文件 / 35 项通过（含新增节点轮询失败保留用例） |
| `npx vitest run src/fleet/fleet.dom.test.jsx src/fleet/useNodes.test.js src/ui/tableLoading.dom.test.jsx src/fleet/executionControls.dom.test.jsx src/fleet/navigation/links.dom.test.jsx src/dag/dag.dom.test.jsx` | 6 文件 / 65 项通过（`EXECUTION_PAGE_LIMIT` 常量同源化后的定向复测） |

## 明确不做（本轮）

- 页面级 URL 深链 / 滚动复位（导航审计 #1/#2，独立迭代，涉及 navPersistence 契约）。
- 写操作反馈三轨统一（表单审计，建议下一轮）。
- 暗色模式、移动端 sticky 导航、其余 13 处轮询收编。
- `spa/dist` 不在本轮重建（共享树有其他在制品，沿用惯例发布前统一执行）。
