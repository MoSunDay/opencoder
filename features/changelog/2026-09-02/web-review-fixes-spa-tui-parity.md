# web 模式审查修复 + SPA 交互对齐 TUI

Commit: af71944

## 背景

- 对 web 面（HTTP 端点 + SSE + SPA）做了一轮审查亲验，发现跨域部署全坏、SSE lag 契约冲突、切换事件不可见、两条静默失败路径、端点边界缺失等问题；同时 SPA 相对 TUI 缺一整层交互（排队/提问/子代理/命令菜单/模型切换）。本轮按 P0→P1→P2→交互对齐顺序收敛。

## 变更

### P0
- `spa/src/api.js`：`signFetch` 改经 `urlFor()` 取 URL——原实现 import 后弃用，跨域登录后除 `/api/time` 外全部签名请求打回同源（登录必败）。
- SSE lag 契约：服务端 `map_broadcast_result` 合成 lag 帧加 `"lag": n` 标记（`api.rs`）；`spa/src/sse.js` 见标记即从持久化 head 重连，无标记的真 run `error` 仍为终态。修复「慢标签页 lag 一次 → 流永久关闭、UI 卡死而 run 还在跑」。

### P1
- `POST /model` / `POST /agent` 成功后经新 `handle.rs::broadcast_persist_event` 广播+落库 `model_switched`/`agent_switched`（TUI parity；`reduce.js` 两个既有 case 从死代码变可达，replay 亦可见）。
- 终帧契约（`handle.rs`）：`resume_session` 失败广播+落库 `error` 帧（行缺失时 FK 落库跳过、仅广播）；run 返回 `Err` 且 runner 未自发 `Error` 时由 `ensure_run_error_frame` 恰好补发一次（`drain_no_restart_on_error` 的 exactly-one-Error 契约不破坏）。

### P2
- `transcript_reset` 前端处理：`reduce.js` 关开文本 turn，`chat.jsx` 收帧即触发 `GET /api/sessions/:id` 快照重载（与 done 同路）。
- 端点边界：`api_questions.rs` 三端点与 `get_event_seq` 对不存在 session 一律 404，且 questions 不再 get-or-create handle（封死 HandleMap 无界增殖）；`/seq` 不再对幽灵 session 返回 `{seq:0}`。
- 清理：`auth_sig_mw.rs` deny body 统一为 `{"ok":false,"error"}`。

### SPA 交互对齐（后端端点全部现成，仅接前端；全部纯函数、无 class）
- 新组件：`queuePanel.jsx`（pending inputs 列表/删除/重排，consumed 帧驱动刷新）、`questionModal.jsx`（仅 live 流 2s 轮询 `/questions`，选项作答/自由文本/跳过）、`subagentBlock.jsx`（`subagent_*` 帧折叠渲染 + `[→ view]` 对 `child_session_id` 开只读回放）、`commandMenu.js`（`/` 命令 + `$` skill 过滤菜单）、`modelModal.jsx`（模型目录 + 切换）。
- `reduce.js`：新增 `subagent_start/child/end`（嵌套 serde 外部 tag 事件经 `nestedEventOf` 归一后复用 reduceFrame 折叠）、`compaction_delta`、`autopilot`、`interrupted`、`transcript_reset` case。
- `chat.jsx`：发送分 `delivery`（Enter=steer、「排队」=queue）；draining 期间 prompt 落在 live session（不再重启流抹掉在途 transcript）；`/act`//plan idle 走 `POST /agent`、busy 改发文本由 runner 边界消费（TUI 排队语义）；`transcript_reset` 快照重载；工具栏 agent Segmented / 模型 / 批注 / 压缩 / autopilot；QueuePanel + QuestionModal 挂载。
- 刻意不做（对齐 TUI 现状）：diff 高亮、sidecar（永不持久化）、tool 输出按 TUI 语义即可。

## Validation（当次实跑）

- `cargo test --workspace`：1918 passed / 0 failed（`schema_v4_migration` 2 例为与并行 cargo 运行的临时文件争用假阳，单跑复绿）。
- `cargo clippy -p opencoder-web --all-targets`：0 警告；改动文件 `rustfmt --check` 干净。
- SPA：`npx vitest run` 13 文件 105 用例全绿；`npm run build` 产出 `dist/static/app.js`（内嵌产物重建）。
- 行数 gate：新文件全部 ≤400 行（最大 `subagentBlock.jsx` 171）；迭代文件 `chat.jsx` 634、`handle.rs` 739、`api.rs` 715 均 ≤800。

## 二轮修复（评审终验 F1–F4）

- **F1 sse.js 旧流不退役**：lag 分支原 `scheduleReconnect()` 后直接 return，旧 readLoop 仍存活，与服务端 Lagged 后继续存活的 merged stream 并发投递（delta 双份渲染、重复 lag 帧叠加重连）。新增 `restart()`：abort 当前连接（AbortError 静默退役）但置 `retired` 标记、不置 `stopped`、不报 `closed`，再换新 AbortController 走 cursor 重连；readLoop 对 `retired` 连接立即停止消费缓冲块（交由 replay 覆盖）。保证任意时刻至多一条活连接。
- **F2 broadcast_persist_event 换序**：`send → append` 改 `append → send`。先落库后广播——两步之间到达的订阅者经 replay 查到行（seq > baseline 播种 overlap 指纹），live 副本被 `sse_dedup::forward_live` 指纹去重吃掉，恰好投递一次；旧序下该订阅者两边都错过。落库失败仍 warn-only 放行 live 帧。
- **F3 远端忙时丢输入**：`chat.jsx send()` 的 `setInput('')` 移到 busy 守卫之后——远端 busy 早退不再静默吞掉已输入的提示词（本地 busy steer 路径语义不变）。注：当前 UI 中 `loading={busy}` 使 Sender 在忙碌时本就不触发 onSubmit，此修复为守卫层防御 + 非 Sender 入口兜底。
- **F4 error 终态 busy 锁死**：done effect 扩为 done/error 双终态复位 busy/connecting（Sender 永久 loading、questionModal 空轮询消除）；transcript 重载保持 done-only。配套 `reduce.js`：带 `lag` 标记的 error 帧视为再同步信号不折叠为终态（否则 F4 会把仍在跑的 turn 提前放行）。
- SPA `dist/` 已重新构建包含以上修复。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| P0-1 跨域 base 前缀 | `signFetch base routing`（2 例） | crates/web/spa/src/api.test.js |
| P0-2 lag 帧带标记 | `lagged_error_carries_numeric_lag_marker` | crates/web/tests/broadcast_lag_handling.rs |
| P0-2 客户端 lag 重连/真错误终态/done 终态 | `reconnects from the seq head after a lag-marked error frame` 等（3 例） | crates/web/spa/src/sse.test.js |
| P1-3 agent/model 切换广播+落库 | `post_agent_broadcasts_and_persists_agent_switched`、`post_model_broadcasts_and_persists_model_switched` | crates/web/tests/switch_broadcast.rs |
| P1-4 run Err 无自发 Error 补发/不重复/Ok 不发 | `run_err_without_runner_error_emits_terminal_error_frame` 等（3 例） | crates/web/src/handle_tests.rs |
| P1-4 resume 失败广播终帧 | `resume_failure_broadcasts_terminal_error_frame` | crates/web/tests/drain_error_frames.rs |
| P1-4 resume 失败不孤儿化订阅者（含先收 error 帧） | `resume_failure_keeps_handle_with_live_subscribers`（更新） | crates/web/tests/handle_resume_failure_keeps_subscribers.rs |
| P2-6 幽灵 session 404 且不增殖 handle | `questions_on_missing_session_is_404_and_creates_no_handle` 等（4 例） | crates/web/tests/web_endpoint_guards.rs |
| reduce 新 case | subagent folding / nestedEventOf / status-line frames（12 例） | crates/web/spa/src/reduce.test.js |
| 交互组件 | commandMenu 11 例、queuePanel 6 例、questionModal 7 例、subagentBlock 6 例 | crates/web/spa/src/{commandMenu.test.js,queuePanel.dom.test.jsx,questionModal.dom.test.jsx,subagentBlock.dom.test.jsx} |
| 回归锚点（未破坏） | `drain_error_never_restarts_and_keeps_inputs_pending`（exactly-one Error）、`web_contract` 15 例、`web_drain_contract` 7 例 | crates/web/tests/* |
| F1 lag 退役旧流/防重连堆叠 | `retires the old connection on lag: post-lag frames drop, reconnects never stack`（负验：改回 scheduleReconnect 即红） | crates/web/spa/src/sse.test.js |
| F2 先落库后广播/失败仍广播 | `broadcast_persists_before_live_send`（负验：换回 send→append 即红）、`broadcast_persist_failure_still_delivers_live` | crates/web/src/handle_tests.rs |
| F4 error 终态复位 busy | `releases the composer on a terminal error frame (busy must not latch)`（负验：仅 done 复位即红） | crates/web/spa/src/chat.dom.test.jsx |
| F4 lag 帧非终态 | `treats a lag-marked error as a re-sync signal, not a terminal failure`（负验：去掉 lag 分支即红） | crates/web/spa/src/reduce.test.js |
| F3 忙碌门复合契约 | `keeps the typed input when a remote run is already busy`（不二次派发、输入不丢） | crates/web/spa/src/chat.dom.test.jsx |
