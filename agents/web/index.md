Commit: (working-tree, sandbox 模式替换 plan/act 双模式)

# web 模块

## 职责
axum HTTP/SSE 会话管理服务。提供 session CRUD、prompt 提交（admit 即返回）、事件流（SSE replay+live）、运行时 agent/model 切换、interrupt；question 作答、queue/steer 输入管理、annotation/autopilot、模型/技能发现、LLM 标题生成（对齐 TUI 会话能力，见 [changelog](../../features/changelog/2026-08-21/web-tui-parity-server-client.md)）。

## 边界与非目标
- 不持有 LLM 客户端单例——每个 prompt 按配置构建 `ChatClient`。
- 非目标：CORS（tower-http 引入但未启用）/ 多 workdir 路由（当前单 workdir）。鉴权**已实现**：全量 HMAC 请求签名中间件（`src/auth_sig_mw.rs`，协议纯函数在 core [agents/core](../core/index.md) `src/auth_sig.rs`）——全部 `/api` 路由要求 `x-sig-timestamp`/`x-sig` 签名（±5min 窗口、同签重放 409、>2MB body 413），豁免 `/`、`/static/*`、`/api/time`、`/favicon.ico`；共享 token 来自 `--token` / `OPENCODER_SERVER_TOKEN` / 自动生成。`serve` 默认仅绑 `127.0.0.1`（回环），需显式 `--host` 才对外。

## 关键抽象
- `AppState`（`src/lib.rs`）：`store: Arc<dyn Store>`、`workdir`、`handles: HandleMap`。
- `SessionHandle`（`src/handle.rs`）：`tx: broadcast::Sender<SseEvt>`、`cancel: Mutex<CancellationToken>`（每次 spawn 刷新，避免上次 interrupt 的永久取消毒化新 drain）、`overrides: Mutex<RuntimeOverrides>`、`draining: AtomicBool`，以及每 session 的 `lifecycle: tokio::sync::Mutex<()>`。所有会改变 agent/model 或把 drain 从 false 启成 true 的入口都必须持有同一 lifecycle 锁，因此“检查 idle → 写 meta/override → 启 drain”与并发请求形成单一全序；按 call_id 的 `child_turn_cancels` / `child_steer_gates` 注册表仍共享给 runner。
- `HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>`：活跃 drain 句柄注册表；handle 可由 `/events` 或 `/prompt` get-or-create。`handle_lifecycle::lock_session_lifecycle` 取得 lifecycle 锁后会复核 map 中仍是同一 `Arc`，handle 被替换时重试，避免锁住已淘汰对象形成双锁域。
- `admit_and_drain_guarded`（`src/handle.rs`）：先 pin handle+lifecycle；若输入带 `agent` 字段 override 且正在 draining，立即返回 `AdmissionError::BusyModeSwitch`，不持久化 skill/input（文本模式命令不再属于 idle-only：正常 admit，由 runner 在边界应用）；否则在锁内完成可选 skill 写入、Store admission 和 `start_drain_locked`。所有 false→true drain 启动（prompt、compact/handoff、restart watcher）集中到 `start_drain_locked`，普通 steer/queue 仍可在运行中 admit 并由 watcher 兜底重启；硬取消后的 pending 不自动复活。
- `drain_to_completion`（`src/handle.rs`）：`resume` 构建 session → 应用 overrides → `run(session, "", ...)`（drain 模式）→ on_event 同时 broadcast + 持久化供 SSE replay。`DrainGuard` 最后释放：事件 sink/flusher 已刷完、`cmd_rx` 已还原之后才把 `draining` 复位，消除尾部仍在写事件/还命令接收器但 API 已观察为 idle 的窗口。**零重提交**：drain 失败即单次 run 终止（无有界重启循环），pending steer/queue 行留在 store 等下一次成功 drain 消费；run 结束后仍应用 endpoint 转发的 drain 命令（autopilot/annotation）。
- MCP 连接池生命周期：session 删除与最后一个 `/events` 订阅者离开时经 `opencoder_session::mcp::cleanup(&id)` 释放该 session 的 MCP 连接；订阅者归零后的 handle eviction 位于 `handle_lifecycle.rs`，同样等待 session lifecycle，不能与 admission/切换并发替换 handle；config reload 时按 `config.enabled_mcp_servers()` 经 `mcp::pool::sync` 增删连接。
- drain 命令通道：`SessionHandle` 另携 `cmd_tx`/`cmd_rx`（`Arc<std::sync::Mutex<CmdRx>>`，锁仅在取命令时短暂持有，绝不跨 await）。`DrainCmd` 枚举（`src/cmd.rs`）：`Compact`、`Handoff{extra}`、`SetSkill(Option<String>)`、`ReloadConfig`、`SetApMode(ApMode)`、`SetAnnotation(Option<String>)`（后三者镜像 TUI worker.rs 语义；plan 阶段的 `ResetPlanPhase` 已随 plan/act 双模式删除）。需 `&mut SessionState`（仅存于 `drain_to_completion` 内）的操作经 `send_cmd()` 入队；`run()` 完成后 `process_drain_cmds()` 在 drain 闭包内排空队列。`CmdRxGuard` 在 Drop 时还原 `cmd_rx`（panic-safe）。
- question hub：`SessionHandle.question_hub`（`Arc<QuestionHub>`）在 handle 上跨 drain 稳定存活；drain/resume 重建 session 时 rebind `session.question_hub` 并 attach——question 工具在 web 下等待作答而非 NO_LISTENER 兜底。**粘性 attach 无 detach**：最后一个 SSE 订阅者断开时 abandon 全部待答问题（工具得 SKIPPED）；多客户端同在线只有最后一个离开才触发。作答入口见 `src/api_questions.rs`（poll 即 attach），abandon/标题生成助手在 `src/handle_questions.rs`。首次 drain run 成功后 best-effort LLM 标题生成（30s 超时，已有 title 跳过）。
- data dir 解析：统一经 `opencoder_core::data_dir_for`（唯一实现与稳定性论证见 [agents/core](../core/index.md)），web 无本地副本。

## 节点面（nodes）
`AppState.nodes: Arc<NodeHub>`（`src/nodes_state.rs`）：task_session_id → `broadcast::Sender<SseEvt>` 映射 + 纯函数 `compute_status`（staleness 20s，按 server 收包时钟记账）。节点端点分居 `api_nodes.rs`（注册表半区：list/register/heartbeat/delete/tasks 派发与列表/cancel）与 `api_nodes_ops.rs`（claim、事件批上传→append_events 带回 seq 后广播、终态上报追加 done/error 收束帧）；浏览器 SSE 桥在 `sse_nodes.rs`，强制复用 `sse_dedup::forward_live` 两级去重（先订阅后查库）。合成 session（task_type="node"）被 `reject_node_session` 在 prompt/agent/model/interrupt/fork/compact/handoff/skill 等 mutation 端点一律 409；`list_sessions` 即使 include_subagents 也排除 node 型。

## 主流程
POST /prompt（`src/api.rs`）：仅 body.agent 属于 idle-only transition（改写会话 agent 配置，draining 时 409 且不写 skill、input、message 或 agent meta）；文本模式命令照常 admit——steer 打断当前 turn 后由 runner 于 turn 边界应用，queue 于 idle 边界应用。cheap busy check 先于 config/client 构建，guarded admission 在 lifecycle 锁内再次裁决；普通 prompt 仍按解析 body → load config → 建 ChatClient → `ensure_session_row` → guarded admit → 返回 `{admitted_seq}`（非阻塞）。
GET /events：`events_after(after)` 重放 + 订阅 broadcast 实时转发（BroadcastStream，lag 客户端丢帧不阻塞 runner）。
GET /api/sessions/:id/seq（`get_event_seq`）：返回该 session 最高已持久化事件 seq（无则 0），供远端 client snapshot 事件游标（只取本次 prompt 产生的事件）。
POST /agent|/model：先 pin session lifecycle，再在同一临界区检查 `draining`、更新 store meta/config 和 handle overrides；drain 已运行时在任何副作用前返回 409，API 检查与 drain 启动之间无 TOCTOU。agent 切换在一个 `SessionPatch` 内原子提交（旧 plan 模式的 `plan_input_count=0` 配套与异步 `ResetPlanPhase` 均已随双模式删除）。`POST /agent` 现只接受 primary `act|sandbox`（legacy `plan` 与子代理名 explore/build 返回 400 unknown-agent，见 `post_agent`）。`POST /model` 的 `persist_default=false` 仍是 session-only，true 才保存全局默认；Config save 失败保留原有回滚。
POST /interrupt：handle.cancel.cancel() → drain 在下个 turn 边界退出。
POST `/api/sessions/:id/subagents/:task_id/steer`：模式控制文本先返回 409 且不 admit child input；普通 steer 再校验 task 属于父 session且为 `Running`，从 live child gate 取得 reservation 后写入并触发 turn cancel。gate 缺失/关闭仍返回 409，写后关闭仍回滚该 row。
- 会话列表（`src/api.rs`）：`GET /api/sessions` 增 `?workdir=` 过滤（经 `opencoder_core::data_dir::workdir_hash`，新会话行打戳；旧 NULL-hash 会话不被匹配）；`POST /agent` 只接受 primary `act|sandbox`、单 `SessionPatch` 原子持久化。事件流端点（`get_events`/`get_event_seq`）位于 `src/api_events.rs`；`GET /events` 支持 `Last-Event-ID` header 回退。
- question 端点（`src/api_questions.rs`）：`GET /api/sessions/:id/questions`（轮询即 attach，返回 waiting `[(call_id, {question, options})]`）、`POST .../questions/:call_id/answer`（body 空 answer → 400）、`POST .../questions/:call_id/skip`（未知 call_id → 404）。
- 输入管理端点（`src/api_inputs.rs`）：`GET /inputs?delivery=queue|steer`（默认 steer，未知会话返回空数组）、`DELETE /inputs/:seq`、`POST /inputs/reorder`。
- annotation/autopilot/模型/技能端点（`src/api_meta.rs`）：`POST /api/sessions/:id/annotation`（`{text}` 设置/空串清空 requirement）、`POST .../autopilot`（`off|ap|review` 会话级 override，`null` 清除、非法值 400）、`GET /api/models`（**脱敏**：永不返回 api_key/headers；按 provider 分组去重下拉列表）、`GET /api/skills`（仅 name/description/enabled）。
- 8 个 feature-parity 端点（`src/api_ops.rs`，于 `src/lib.rs::build_app()` 注册）：fork、compact、handoff、skill、config GET/PATCH、bg list/stop。compact 可在运行中排入但经 lifecycle 与 drain 启动串行化；handoff 会切模式，只允许 idle，running 时 409 且不发命令/不启动 drain。
- 子代理观测 + clear-all（`src/api_subagents.rs`，2026-08-25）：`GET /api/sessions/:id/subagents` 列子代理任务（`kind` 映射存储 `agent`；空列表 200、无会话 404）；`DELETE /api/sessions?keep=:id` 照抄 TUI `gate_clear_all`——任一 live handle draining（含 keep 自身）→ 409，否则非 keep handle 走 delete_session 同款 evict 后 `clear_other_sessions` FK 级联（缺参 400、keep 不存在 404、幂等 `removed:0`）。
- `/api/envs` 环境配置管理（`src/api_envs.rs`，5 路由）：`GET`（列表 + active）、`POST {name, capture_current=true}`（400 非法名 / 409 重名）、`PATCH {active: name|null}`（404 未知环境）、`POST /:name/recapture`、`DELETE /:name`（active 环境删除先清标记）。所有会改变有效配置的变更向全部 live session 扇出 `DrainCmd::ReloadConfig`（与 `PATCH /api/config` 同机制——快照 handles keys 后逐个 send_cmd）；recapture/delete 仅在影响 active 环境时扇出。`GET /api/config` 自动反映环境层（`Config::load` 解析）。
- SPA 前端 `GET /`（`src/html.rs`）：React18 + antd + Vite 源码在 `spa/src`（单测 vitest），构建产物**提交**在 `spa/dist`（`index.html` + `static/app.js`/`static/app.css` 固定文件名、无内容哈希），经 `include_str!`/`include_bytes!` 编译期内嵌——`cargo build` 永不需要 node；路由 `/` + `/static/:name` 白名单伺服（白名单外 404）；构建 `scripts/build-spa.sh`、漂移检查 `scripts/check-spa-drift.sh`。所有 `/api` 调用经 `spa/src/sign.js` 按共享 token 携带 `x-sig-timestamp`/`x-sig` 签名；`crypto.subtle` 只在 secure context 存在（LAN IP 明文 `http://` 下为 undefined，签名必炸）——`sign.js` 探测缺失即回退 `spa/src/sha256.js` 纯 JS SHA-256/HMAC（向量对齐 NIST/RFC4231 与 Rust 侧常量），内网 HTTP 原生可用、明文警示 banner 已移除；`GET /api/time`（`server_time_ms`）供浏览器校准时钟偏移。SSE 重连契约（`spa/src/sse.js`）：按 `?after=lastSeq` 做**有界尾部重放**，`REPLAY_CAP_FRAMES=400` 封顶且终帧恒为 head（无封顶时 4 万帧回放 O(n²) 会冻结页面；整体 resync 机制已删除）；`GET /api/sessions/:id` 响应含 `draining: bool`（compact 等长收尾可观测）；wire `ContentBlock` 是 serde `tag = "kind"`（`crates/core/src/message.rs`），SPA 快照渲染兼容 `kind`/`type` 双 tag。

## 依赖与接口
- 依赖：axum 0.7（ws feature）、tokio-stream（sync feature，BroadcastStream）、tokio-util（CancellationToken）、opencoder-session/store/llm/core。
- 被依赖：cli（serve 命令）。

## 相关模块
- [agents/session](../session/index.md) — drain 与 cancel。
- [agents/store](../store/index.md) — 持久化与事件回放。

## 代表性锚点
- HTTP 表面契约测试：`tests/web_contract.rs`（health、session CRUD、prompt admit 立即返回、SSE replay+live、agent/model 切换持久化、interrupt 取消 token）
- drain 生命周期契约测试：`tests/web_drain_contract.rs`（pre-existing handle 不阻塞 drain 的 F1 回归；drain 完成后 `draining` 复位使再次 prompt 重 spawn 的 G1；interrupt 不毒化后续 drain 的 G2；早订阅者经共享 broadcast 收 live 的 G3）
- drain 生命周期测试：`tests/web_drain_contract.rs`（早订阅 handle 不阻塞 drain、drain 完成后再次 prompt 再 spawn、interrupt 后再 prompt 跑到完、先订 /events 再 prompt 收 live 帧、POST /prompt 配置失败→500、/events 慢订阅者背压）
- feature-parity 端点测试：`tests/web_api_ops.rs`（fork/skill/compact/handoff/config/bg）；SPA 装配单测：`src/html.rs`。 环境管理端点：`tests/web_envs.rs`（列表/创建/激活/重捕获/删除 + ReloadConfig 扇出）。子代理列表/clear-all（2026-08-25）：`tests/subagent_list_api.rs`、`tests/clear_sessions_api.rs`；SPA 单测在 `spa/src`（vitest：sign/reduce）。
- web/TUI 对齐端点测试（2026-08-21）：`tests/web_questions.rs`（answer 闭环/skip→SKIPPED/400/404/空列表/最后订阅者断开 abandon）、`tests/web_inputs.rs`（queue 列表/删除/重排 + 默认 steer）、`tests/web_meta_endpoints.rs`（annotation/autopilot/models 脱敏/skills 形状）、`tests/web_list_events.rs`（workdir 过滤 + Last-Event-ID replay）、`tests/web_drain_cmds.rs`（SetApMode/SetAnnotation live 应用 + 持久化）、`tests/client_remote_ops.rs`（client 侧 18 方法 e2e）。
- running 模式门：`tests/running_mode_gate.rs`（真实 router + 挂起 LLM：agent 字段与 handoff 运行中 409 且无副作用；文本模式命令 queue/steer 运行中 200，分别在 idle/turn 边界应用生效）、`tests/agent_model_toctou.rs`（drain 与 agent/model 的双向 lifecycle 排序）、`tests/subagent_steer_api.rs`（mode steer 拒绝）、根目录 `tests/running_mode_switch_e2e.rs`（真实 `opencoder daemon --server` 二进制和 HTTP 阻塞 provider）。handle identity/eviction 单测位于 `src/handle_tests.rs`。
