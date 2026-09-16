Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# 运行画布点选步骤改开「节点侧单步执行记录」抽屉

舰队控制台 DAG 运行详情此前点任一 step 打开的是运行全量日志抽屉（LogsDrawer，按 step 过滤）。后端已提供单步事件流 `GET /api/dag/runs/:rid/steps/:step/events`（帧 `{event,data,seq}`：wasm 步为节点侧解析的 `step_output` 捕获，agent 步直通子会话 `sse_kind`，终态追加 `step_finished`）与单步回执 `GET /api/dag/runs/:rid/steps/:step`。本次在 SPA 新增 `spa/src/dag/step/` 面板族接入这两个契约，Rust 零改动。

## 变更内容

- 新建 `spa/src/dag/step/`（8 文件，全部 ≤400 行）：
  - `model.js` 纯投影：`outputRows`（step_output 扁平帧与 step_log 嵌套镜像 → {seq,at,stream,label,text} 行，相邻同 stream 合并、query 大小写不敏感过滤）、`finishedOf`（末帧 step_finished 回执）、`STEP_KIND_LABEL`/`isAgentKind`。
  - `useStepStream.js`：单流同时维护 appendLog 帧窗口与 reduceExecutionFrame TUI 转写；openStream 走 executionHistory+requireEnd，onResync 以游标为地板；failed 不被 closed 覆盖；runId/step 变更或 retry 全量重置重连。
  - `wasmLogs.jsx`/`agentTranscript.jsx`/`stepPanel.jsx`：wasm 步渲染 [stdout]/[stderr] 前缀合并日志（复用 run.css 的 .execution-log-lines，不新建 css），agent 步复用 TranscriptView 折叠 Say/工具阶梯；step_finished 优先于会话 status/error；连接失败 Alert 带「重新连接」。
  - `stepDrawer.jsx`：75vw 右抽屉（rootClassName=dag-logs-drawer），头部 Descriptions 展示回执 类型/状态/开始/结束/会话（session_id copyable），extra 三钮 刷新/运行日志/关闭；onFinished 与刷新都重拉回执，回执 status 缺失回落 finished.status。
- `spa/src/dag/run/result.jsx` 接线：点 step → StepDrawer（specKind 作首屏回落），新增 logsOpen state，「运行日志」再开原 LogsDrawer（props/行为不变）。
- `spa/src/ui/executionEvents/model.js`：logEntry 把扁平 step_output 帧投影为 stdout/stderr 事件，使运行全量日志抽屉里 wasm 输出天然获得 LABELS 与相邻合并；model.test.js 补该投影用例。

## Impact Surface

- 新增 `crates/web/spa/src/dag/step/{model.js,model.test.js,useStepStream.js,wasmLogs.jsx,agentTranscript.jsx,stepPanel.jsx,stepDrawer.jsx,step.dom.test.jsx}`
- 修改 `crates/web/spa/src/dag/run/result.jsx`、`crates/web/spa/src/dag/graph.dom.test.jsx`（点选步骤断言改走 StepDrawer→运行日志 两级）、`crates/web/spa/src/ui/executionEvents/{model.js,model.test.js}`
- Rust / 协议 / dist 零改动

## 测试覆盖

| 契约 | 用例 | 文件 |
| --- | --- | --- |
| outputRows 合并/嵌套兼容/过滤、finishedOf、kind 判定 | 5 个纯函数用例 | `src/dag/step/model.test.js` |
| wasm 步渲染合并 stdout/stderr 行 | `renders wasm stdout/stderr rows and merges adjacent stdout fragments` | `src/dag/step/step.dom.test.jsx` |
| agent 步 text_delta+tool_start/end 折叠为 Say/工具阶梯 | `folds agent child-session frames into a TUI say/tool transcript` | `src/dag/step/step.dom.test.jsx` |
| 抽屉回执 类型/状态/会话 展示；step_finished 触发重拉并回落 finished.status；运行日志/关闭回调 | `shows the receipt kind, status and session in the drawer and opens the run logs` | `src/dag/step/step.dom.test.jsx` |
| step_output 投影进 logEntry/logRows（相邻合并、stderr 独立行） | `projects node-side step_output frames onto merged stdout/stderr rows` | `src/ui/executionEvents/model.test.js` |
| 画布点选 → StepDrawer，运行日志二级打开/关闭 | `shows final states immediately, opens the step drawer and loads run logs behind it` | `src/dag/graph.dom.test.jsx` |

## 验证

- `npx vitest run src/dag src/ui/executionEvents` → 12 files / 95 tests 全绿
- `npm test` → 104 files / 737 tests 全绿（基线 101/712 + 工作树在途改动）
