Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# DAG 单步执行记录：查询链路 + 控制台抽屉 + 单一写者守卫

## Context

DAG 运行页此前只有「运行全量日志」一条观察路径：它按 `payload.step` 过滤 **run 会话**（Node Store `session_events`）后分页/SSE 回放。这条路径天生看不到 agent 步的真实执行痕迹——agent 步的 `tool_start`/`tool_end`/say-reason 等 TUI 级帧只落在**子会话**上，run 会话里只有被上报回来的 `step_log`（且只有 `text_delta`）。用户想知道「这一步在这个节点上到底干了什么、现在跑到哪」只能在全量日志里搜步骤名。

节点侧的数据生产已就绪（`session.json` 实时指针、`meta.json.session_id` 终态兜底、run 会话上的 `step_output` 镜像，见 [2026-09-11/dag-step-records-node-side.md](../2026-09-11/dag-step-records-node-side.md)），缺的是「点 step → 查该节点的单步执行记录 → 运行中实时渲染」的查询与传输入口。传输沿用仓库既有的浏览器 SSE（`EventSource` + `Last-Event-ID` 重连）：全仓没有浏览器侧 WebSocket，本次不新增传输形态。

## Change Summary

- **线协议 additive 扩展**（`crates/core/src/fleet/protocol.rs`）：新增 `NodeOperation::DagStepEvents { execution, step, after }`，`after` 带 `#[serde(default)]`（缺省 0，等价于从头回放）；`PROTOCOL_VERSION` 仍是 9，既有 `dag_steps` 线形状一字未动。
- **节点单步事件查询**（新增 `crates/worker/src/operations/query/dag_step_events.rs`，`operations/mod.rs` 分派）：
  - *事件源选择*：`kind == "agent"` 且能解析出子会话（`session.json` 优先、`meta.json.session_id` 兜底）→ 直接读子会话，**全保真**、不过滤；否则读 run 会话并用 `StepFilter` 按 `payload.step` 过滤，wasm 步再限定 `sse_kind ∈ {step_output, step_log}`，一步绝不会继承另一步的 stdout/stderr。
  - *游标推进*：整页被过滤掉也推进扫描游标，单次 poll 最多扫 `MAX_FILTER_PAGES = 8` 页；封顶后 `more = true`，下一次 poll 从游标继续——「这一步还没输出」不会让一次 poll 回放整个 run。
  - *分页/字节守卫*与 `query::events` 完全一致（`EVENT_PAGE_MAX`、`QUERY_RESPONSE_BYTES - 64 KiB`、单帧超 `MAX_FRAME_BYTES - ENVELOPE_RESERVE(128 KiB)` 回 413、单页最多 200 帧），因此 control 的分页→SSE 循环可原样复用。
  - *终态帧*：`more == false` 且回执状态 ∈ {done, error, cancelled} 时追加 `step_finished`（`{status, error, started_at_ms, finished_at_ms}`，seq = 已发帧最大 seq + 1），错误文本按既有 `truncate_error_ref` 限长；未收完分页时不发终态帧。子会话尚未创建**不是**错误：`step` 视图照常回答，流继续 poll。
  - *回执*：body 里的 `step` 视图带 `name/kind/status/error/started_at_ms/finished_at_ms` 与可选 `session_id`；未知 run、spec 里没有的 step、非 DAG execution 分别 404/404/400。
- **单步回执扩展**（`query/dag_steps.rs`）：`GET /api/dag/runs/:id/steps/:step` 响应体新增 `kind`（取自 run spec）与 `session_id`（同一套 `session.json` → `meta.json` 解析），抽屉头部与「查看会话」入口据此渲染。
- **控制面 SSE**（`crates/control/src/api/stream.rs::dag_step_events` + `api/compat/mod.rs` 路由 `GET /api/dag/runs/:id/steps/:step/events`，`api/executions/mod.rs::dag_step_events_id` 派发）：与 `GET /api/dag/runs/:id/events` **共用同一个** `sse()` 分页循环（`?after=` 与 `Last-Event-ID` 取大者作游标、300 ms 轮询、5 s keep-alive、`stream_end` 收尾、首页非 200 直接透传、轮询错误折成一个 `error` 帧后终止），事件源完全由节点决定。路由留在 compat DAG 面，角色矩阵未放开 → admin-only。
- **控制台抽屉**（SPA `src/dag/step/`，8 文件 ≤400 行，详见 [spa-dag-step-record-drawer.md](spa-dag-step-record-drawer.md)）：`model.js` 纯投影（`outputRows`/`finishedOf`/kind 判定）、`useStepStream.js` 单流双投影（同一个 `onFrame` 同时喂 `appendLog` 帧窗口与 `reduceExecutionFrame` TUI 转写，wasm 日志视图与 agent 转写不会漂移）、`wasmLogs.jsx`（实时日志）/`agentTranscript.jsx`（类 TUI 会话转写）/`stepPanel.jsx`/`stepDrawer.jsx`（回执 + 刷新/运行日志/关闭）。`dag/run/result.jsx` 点 step 由「运行日志过滤」升级为「单步记录抽屉」，运行全量日志保留为抽屉内二级入口（`LogsDrawer` props/行为不变）。
- **全量日志词汇对齐**（`src/ui/executionEvents/model.js`）：`logEntry` 把节点侧扁平 `step_output` 帧投影成 stdout/stderr 事件，运行全量日志里的 wasm 输出因此天然获得 LABELS 与相邻同流合并。

## 收敛决策：`step_output` 与 `step_log` 不合并

初始计划是删掉 `step_log.rs` 的 `step_output` 管道、统一到 `step_log`（run 级 Uplink 上报 + `workloads/dag.rs::LocalDagPersistence::events` 的本地镜像），理由是「同一份 stdout 看起来被写了两次」。核查后确认**不存在 run 会话双写**，两条链路按 step kind 互斥：

- **wasm 步**：`StepOutputLog` 把 stdout/stderr 直接写进 Node Store 的 run 会话（`session_id = run_id`、`sse_kind = "step_output"`、payload `{step,stream,text,at_ms}`），接线在 `runtime.rs::execute_step`（in-process sink 镜像 + `sandbox/runc.rs` 管道 tee）。wasm 执行器**从不**使用 `ctx.log`；`exec/logs.rs::StepLog::output` 至今没有生产调用方（唯一调用点 `sandbox/output_limit.rs::read_logged` 的 `log` 形参在 `sandbox/runc.rs` 传的是 `None`）。
- **agent 步**：`exec/agent.rs` 只把 `TextDelta` 经 `ctx.log.text_delta()` 发成 run 级 `kind = "step_log"` 事件（payload `{event:"text_delta",data:{text}}`），由 `dag_events.rs` 批量上报、`LocalDagPersistence::events` 镜像回 run 会话（`sse_kind = e.kind`、payload `{kind,step,payload,at_ms}`）。agent 步**不写** `step_output`。

结论：两条链路都保留——`step_output` 是 wasm 输出的唯一写者，`step_log` 是 agent `text_delta` 的唯一写者，运行全量日志与单步记录都不会出现重复行。查询侧 `StepFilter` 同时接受这两个 `sse_kind`，是为兼容 runner 步与历史行，**不是**为了去重。为防止后人「顺手补一条」造成真双写，`crates/dag-runtime/tests/run_loop.rs::wasm_step_output_is_mirrored_to_the_run_session` 末尾新增**单一写者守卫**：wasm 步既不上报任何 `step_log` 事件（stub 捕获 `count_events(&c, "step_log", "build") == 0`），run 会话里也没有 `step_log` 行（`rows` 中 `sse_kind == "step_log"` 计数为 0）。守卫非空转：同文件 `single_agent_step_completes_and_reports_done` 用同一套 stub 捕获断言 agent 步**确实**产出 `step_log`（`payload.event == "text_delta"`），即「wasm=0 / agent>0」的互斥被两端同时钉住。

## Impact Surface

- **节点 ↔ 控制面协议**：additive，`PROTOCOL_VERSION` 未升。握手仍按 v9 强校验（`transport/hub.rs` 拒绝异代际注册），所以「旧 server + 新节点」= 新 operation 永不发出、节点行为不变；「新 server + 同代际但更旧的节点构建」派发该 operation 时，旧节点无法反序列化 `ServerFrame::Call`，出站通道按既有逻辑报错重连（不会静默挂死），控制台侧表现为一个 `error` 帧。
- **DAG 运行页交互**：点 step 打开单步记录抽屉（原为运行日志抽屉），运行全量日志降为抽屉内二级入口；`LogsDrawer` 自身未变。
- **权限**：新路由继承 compat DAG 面的 admin-only 约束，非 admin 403（`role_gate` 纯函数用例 + e2e 各钉一处）。
- **无 DB schema 变更**（只读 `session_events`）、无工件格式变更、无 dist 手工编辑（`crates/web/spa/dist/` 由 `scripts/build-spa.sh` 重建）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| `DagStepEvents` 线形状 additive（`after` 缺省为 0、`dag_steps` 形状不变） | `fleet::protocol::tests::dag_step_events_wire_shape_is_additive` | `crates/core/src/fleet/protocol.rs` |
| agent 步直通子会话全保真；`step` 视图带 `kind`/`session_id` | `operations::query::tests::dag_step_events::sources::dag_step_events_agent_step_streams_its_child_session` | `crates/worker/src/operations/query/tests/dag_step_events/sources.rs` |
| wasm 步只保留自己的 `step_output`；wasm 步无 `session_id` | `...::sources::dag_step_events_wasm_step_keeps_only_its_own_step_output` | 同上 |
| runner 步只按 `payload.step` 过滤（不限 `sse_kind`） | `...::sources::dag_step_events_runner_step_filters_by_step_only` | 同上 |
| 子会话未建时继续 poll，`session.json` 落盘后切源并补 `step_finished` | `...::sources::dag_step_events_keeps_polling_before_the_step_session_exists` | 同上 |
| 整页被过滤仍推进游标 | `...::paging::dag_step_events_scans_past_a_fully_filtered_page` | `crates/worker/src/operations/query/tests/dag_step_events/paging.rs` |
| `MAX_FILTER_PAGES` 封顶后如实报 `more` | `...::paging::dag_step_events_caps_the_filter_scan_and_reports_more` | 同上 |
| 终态追加 `step_finished`（status/error/起止时间） | `...::paging::dag_step_events_terminal_step_appends_step_finished` | 同上 |
| 未收完分页不发终态帧 | `...::paging::dag_step_events_withholds_step_finished_until_the_last_page` | 同上 |
| 未知 run / 未知 step / 非 DAG kind 的拒绝码 | `...::paging::dag_step_events_rejects_unknown_runs_steps_and_kinds` | 同上 |
| SSE 回放 step 帧并在 `step_finished` 后关流（`id:` 游标随帧下发） | `dag_step_events_replay_the_step_stream_and_close_on_finished` | `crates/control/tests/e2e/dag_step_events.rs` |
| `?after=` 与 `Last-Event-ID` 双通道游标续传 | `dag_step_events_resume_from_after_and_last_event_id` | 同上 |
| `more` 保持轮询不提前结束 | `dag_step_events_more_flag_keeps_the_stream_polling` | 同上 |
| 首页错误透传（非 200 不进 SSE） | `dag_step_events_first_page_errors_pass_through` | 同上 |
| 角色门禁：非 admin 拒绝 | `dag_step_events_are_admin_only` | 同上 |
| 路由矩阵：admin 全通、非 admin 不含 step 事件流 | `role_gate::tests::{admin_keeps_the_full_surface, non_admins_read_identity_nodes_and_executions}` | `crates/control/src/role_gate.rs` |
| **单一写者守卫**：wasm 步只有 `step_output`，无 `step_log` 事件、run 会话无 `step_log` 行 | `run_loop::wasm_step_output_is_mirrored_to_the_run_session` | `crates/dag-runtime/tests/run_loop.rs` |
| agent 步 `session.json` == `session_value`、与 `meta.json.session_id` 一致、子会话真实存在 | `run_loop::single_agent_step_completes_and_reports_done` | 同上 |
| 单步进度/回执视图（`kind`+`session_id` 所在响应体） | `dag_run_progress_and_step_views_after_completion`、`dag_run_progress_reports_running_step_while_in_flight` | `crates/worker/tests/platform/dag_run_steps.rs` |
| SPA 纯投影：相邻同流合并/嵌套 `step_log` 镜像/大小写不敏感过滤/`finishedOf`/kind 判定（describe `dag step projections`） | `merges adjacent stdout fragments and never merges across streams`、`reads nested step_log mirrors and ignores non-output kinds`、`filters rows with a case-insensitive substring query`、`reports the last terminal receipt from finishedOf`、`labels step kinds and detects agent steps` | `crates/web/spa/src/dag/step/model.test.js` |
| SPA wasm 步渲染合并 stdout/stderr 行 | `renders wasm stdout/stderr rows and merges adjacent stdout fragments` | `crates/web/spa/src/dag/step/step.dom.test.jsx` |
| SPA agent 步 text_delta + tool_start/end 折叠为 TUI say/工具阶梯 | `folds agent child-session frames into a TUI say/tool transcript` | 同上 |
| SPA 抽屉回执（类型/状态/会话）、`step_finished` 重拉、运行日志二级入口 | `shows the receipt kind, status and session in the drawer and opens the run logs` | 同上 |
| SPA 全量日志把 `step_output` 投影成 stdout/stderr 词汇（describe `execution log projection`） | `projects node-side step_output frames onto merged stdout/stderr rows` | `crates/web/spa/src/ui/executionEvents/model.test.js` |
| SPA 画布点选 → 单步抽屉 → 运行日志二级打开 | `shows final states immediately, opens the step drawer and loads run logs behind it` | `crates/web/spa/src/dag/graph.dom.test.jsx` |

## 验证

- `cargo test -p opencoder-dag-runtime --test run_loop` → `test result: ok. 8 passed; 0 failed; 0 ignored`（7.24s），含新增单一写者守卫的 `wasm_step_output_is_mirrored_to_the_run_session` 与 `single_agent_step_completes_and_reports_done`。
- `cargo clippy -p opencoder-dag-runtime --all-targets -- -D warnings` → 零告警（`Finished dev profile in 27.31s`，退出码 0）。
- 环境说明：共享 `CARGO_TARGET_DIR=/data00/rust-build/cargo/default` 被并行会话长期持有文件锁（机器 load > 150），本次两条命令均在锁释放后执行；首轮 clippy 曾因并行会话正在改 `crates/agents`（manifest 解析竞态）失败，与本次改动无关，重跑即绿。
- 全量回归 `cargo test --workspace` 与 SPA `npm test` 由本轮迭代收尾统一执行（规则 `rules/02-regression-gate.md`）。

## 相关

- [节点侧数据生产](../2026-09-11/dag-step-records-node-side.md)
- [SPA 单步记录抽屉](spa-dag-step-record-drawer.md)
- [agents/dag-runtime](../../../agents/dag-runtime/index.md)、[agents/worker](../../../agents/worker/index.md)、[agents/control](../../../agents/control/index.md)
