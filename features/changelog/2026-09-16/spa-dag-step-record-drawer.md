Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# 运行画布点选步骤改开「节点侧单步执行记录」抽屉


## 变更内容

- 新建 `spa/src/dag/step/`（8 文件，全部 ≤400 行）：
  - `model.js` 纯投影：`outputRows`（step_output 扁平帧与 step_log 嵌套镜像 → {seq,at,stream,label,text} 行，相邻同 stream 合并、query 大小写不敏感过滤）、`finishedOf`（末帧 step_finished 回执）、`STEP_KIND_LABEL`/`isAgentKind`。
  - `useStepStream.js`：单流同时维护 appendLog 帧窗口与 reduceExecutionFrame TUI 转写；openStream 走 executionHistory+requireEnd，onResync 以游标为地板；failed 不被 closed 覆盖；runId/step 变更或 retry 全量重置重连。
  - `stepDrawer.jsx`：75vw 右抽屉（rootClassName=dag-logs-drawer），头部 Descriptions 展示回执 类型/状态/开始/结束/会话（session_id copyable），extra 三钮 刷新/运行日志/关闭；onFinished 与刷新都重拉回执，回执 status 缺失回落 finished.status。
- `spa/src/dag/run/result.jsx` 接线：点 step → StepDrawer（specKind 作首屏回落），新增 logsOpen state，「运行日志」再开原 LogsDrawer（props/行为不变）。

## Impact Surface

- 修改 `crates/web/spa/src/dag/run/result.jsx`、`crates/web/spa/src/dag/graph.dom.test.jsx`（点选步骤断言改走 StepDrawer→运行日志 两级）、`crates/web/spa/src/ui/executionEvents/{model.js,model.test.js}`
- Rust / 协议 / dist 零改动

## 测试覆盖

| 契约 | 用例 | 文件 |
| --- | --- | --- |
| outputRows 合并/嵌套兼容/过滤、finishedOf、kind 判定 | 5 个纯函数用例 | `src/dag/step/model.test.js` |
| agent 步 text_delta+tool_start/end 折叠为 Say/工具阶梯 | `folds agent child-session frames into a TUI say/tool transcript` | `src/dag/step/step.dom.test.jsx` |
| 抽屉回执 类型/状态/会话 展示；step_finished 触发重拉并回落 finished.status；运行日志/关闭回调 | `shows the receipt kind, status and session in the drawer and opens the run logs` | `src/dag/step/step.dom.test.jsx` |
| step_output 投影进 logEntry/logRows（相邻合并、stderr 独立行） | `projects node-side step_output frames onto merged stdout/stderr rows` | `src/ui/executionEvents/model.test.js` |
| 画布点选 → StepDrawer，运行日志二级打开/关闭 | `shows final states immediately, opens the step drawer and loads run logs behind it` | `src/dag/graph.dom.test.jsx` |

## 验证

- `npx vitest run src/dag src/ui/executionEvents` → 12 files / 95 tests 全绿
- `npm test` → 104 files / 737 tests 全绿（基线 101/712 + 工作树在途改动）
