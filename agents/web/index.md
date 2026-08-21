Commit: 860831d22fad968737c366c93b4cf70fc1f4c010

# web 模块

## 职责
axum HTTP/SSE 会话管理服务。提供 session CRUD、prompt 提交（admit 即返回）、事件流（SSE replay+live）、运行时 agent/model 切换、interrupt；question 作答、queue/steer 输入管理、annotation/autopilot、模型/技能发现、LLM 标题生成（对齐 TUI 会话能力，见 [changelog](../../features/changelog/2026-08-21/web-tui-parity-server-client.md)）。

## 边界与非目标
- 不持有 LLM 客户端单例——每个 prompt 按配置构建 `ChatClient`。
- 非目标：CORS（tower-http 引入但未启用）/ 多 workdir 路由（当前单 workdir）。鉴权**已实现**（`auth.rs`：bearer token 中间件，token 来自 `--token` / `OPENCODER_SERVER_TOKEN` / 自动生成）。`serve` 默认仅绑 `127.0.0.1`（回环），需显式 `--host` 才对外。

## 关键抽象
- `AppState`（`src/lib.rs`）：`store: Arc<dyn Store>`、`workdir`、`handles: HandleMap`。
- `SessionHandle`（`src/handle.rs`）：`tx: broadcast::Sender<SseEvt>`、`cancel: Mutex<CancellationToken>`（每次 spawn 刷新，避免上次 interrupt 的永久取消毒化新 drain）、`overrides: Mutex<RuntimeOverrides>`、`draining: AtomicBool`（CAS 标记 drain 是否在跑，**与 map 存在性解耦**——否则早订阅的 `/events` 句柄会阻止 drain spawn），以及按 call_id 共享给 session runner 的 `child_turn_cancels` / `child_steer_gates` 注册表。
- `HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>`：活跃 drain 句柄注册表；handle 可由 `/events` 或 `/prompt` get-or-create。
- `admit_and_drain`（`src/handle.rs`）：admit 输入到 Store → get-or-create handle（共享 broadcast 通道）→ `draining.swap(true)` CAS 决定是否 spawn drain → 立即返回 admitted_seq。运行中 admit（steer/queue 落入 else 分支）另起 watcher：轮询至 drain 退出后复查 pending 兜底重启（换新 cancel token）——但 drain 是被硬取消（POST /interrupt、DELETE /session）打断时**绝不复活**（读当前 token `is_cancelled()` 判定，用户停止语义优先，pending 行留给下次用户主动 drain）。
- `drain_to_completion`（`src/handle.rs`）：`DrainGuard` 在 Drop（含 panic）复位 `draining`；`resume` 构建 session → 应用 overrides → `run(session, "", ...)`（drain 模式）→ on_event 同时 broadcast（`SseEvt::from_session_event` 现走 `SessionEvent::sse_kind()/sse_data()/coarse_kind()` 单一真相源）+ 落 `session_events` 表（持久化 `sse_kind`）供 SSE replay；`GET /events` 的 `get_events` replay 优先取 `sse_kind`、`None` 回退 `event_kind_str(coarse)` ；run Err 且仍有 pending 输入且未被硬取消时有界重启（`MAX_DRAIN_RESTARTS=2`，250ms 退避；store 读错误按零 pending 处理不误判复活，硬取消语义优先）→ 完成后**保留 handle 于 map**（供 late SSE replay + 后续 re-admit 再 spawn）；仅 resume 失败（session 行缺失）时移除。
- MCP 连接池生命周期：session 删除与最后一个 `/events` 订阅者离开时经 `opencoder_session::mcp::cleanup(&id)` 释放该 session 的 MCP 连接；订阅者增减经 `release_events_subscriber`（`src/handle.rs`——创建者先离开时句柄仍保留给其余订阅者，计数归零才 evict+cleanup）；config reload 时按 `config.enabled_mcp_servers()` 经 `mcp::pool::sync` 增删连接。
- drain 命令通道：`SessionHandle` 另携 `cmd_tx`/`cmd_rx`（`Arc<std::sync::Mutex<CmdRx>>`，锁仅在取命令时短暂持有，绝不跨 await）。`DrainCmd` 枚举（`src/cmd.rs`）：`Compact`、`Handoff{extra}`、`SetSkill(Option<String>)`、`ReloadConfig`、`SetApMode(ApMode)`、`SetAnnotation(Option<String>)`、`ResetPlanPhase`（后三者镜像 TUI worker.rs 语义）。需 `&mut SessionState`（仅存于 `drain_to_completion` 内）的操作经 `send_cmd()` 入队；`run()` 完成后 `process_drain_cmds()` 在 drain 闭包内排空队列。`CmdRxGuard` 在 Drop 时还原 `cmd_rx`（panic-safe）。
- question hub：`SessionHandle.question_hub`（`Arc<QuestionHub>`）在 handle 上跨 drain 稳定存活；drain/resume 重建 session 时 rebind `session.question_hub` 并 attach——question 工具在 web 下等待作答而非 NO_LISTENER 兜底。**粘性 attach 无 detach**：最后一个 SSE 订阅者断开时 abandon 全部待答问题（工具得 SKIPPED）；多客户端同在线只有最后一个离开才触发。作答入口见 `src/api_questions.rs`（poll 即 attach），abandon/标题生成助手在 `src/handle_questions.rs`。首次 drain run 成功后 best-effort LLM 标题生成（30s 超时，已有 title 跳过）。
- data dir 解析：统一经 `opencoder_core::data_dir_for`（唯一实现与稳定性论证见 [agents/core](../core/index.md)），web 无本地副本。

## 主流程
POST /prompt（`src/api.rs`）：解析 body → load config → 建 ChatClient → `ensure_session_row` → `admit_and_drain` → 返回 `{admitted_seq}`（非阻塞）。
GET /events：`events_after(after)` 重放 + 订阅 broadcast 实时转发（BroadcastStream，lag 客户端丢帧不阻塞 runner）。
GET /api/sessions/:id/seq（`get_event_seq`）：返回该 session 最高已持久化事件 seq（无则 0），供远端 client snapshot 事件游标（只取本次 prompt 产生的事件）。
POST /agent|/model：更新 store meta + handle.overrides（下一轮 drain 生效）。**RUNNING-GATE**：`draining` 为真（drain 进行中）时两者在**任何** store-meta/config/override 变更之前返回 409（`error_409`：`agent switch refused while drain running` / `model switch refused while drain running`），get-or-create handle 先行保证 override 永不被丢弃（原子性，镜像 `post_interrupt` 的 draining-gate）。`POST /model` body 新增 `persist_default: bool`（`#[serde(default)]`=false）：false（默认）= session-only（**不写盘**，与 TUI `/model` 默认一致）；true=额外 `Config::save` 写全局默认到 opencoder.json，失败返 500。
POST /interrupt：handle.cancel.cancel() → drain 在下个 turn 边界退出。
POST `/api/sessions/:id/subagents/:task_id/steer`：先校验 task 属于父 session 且为 `Running`，再从 live handle 的 child gate 取得 reservation 后写入 child session。gate 缺失/关闭返回 409 且不落 input；若写入后被强制关闭则删除该 row 后返回 409；成功提交后触发目标 child turn cancel，保持 Web 的立即打断语义。
- 会话列表/事件流（`src/api.rs`）：`GET /api/sessions` 增 `?workdir=` 过滤（经 `opencoder_core::data_dir::workdir_hash`，新会话行打戳；旧 NULL-hash 会话不被匹配）；`GET /events` 增 `Last-Event-ID` header 回退（无 `?after=` 时读 header）；`POST /agent` 切到 plan 时发 `ResetPlanPhase` 并持久化 `plan_input_count=0`。
- question 端点（`src/api_questions.rs`）：`GET /api/sessions/:id/questions`（轮询即 attach，返回 waiting `[(call_id, {question, options})]`）、`POST .../questions/:call_id/answer`（body 空 answer → 400）、`POST .../questions/:call_id/skip`（未知 call_id → 404）。
- 输入管理端点（`src/api_inputs.rs`）：`GET /inputs?delivery=queue|steer`（默认 steer，未知会话返回空数组）、`DELETE /inputs/:seq`、`POST /inputs/reorder`。
- annotation/autopilot/模型/技能端点（`src/api_meta.rs`）：`POST /api/sessions/:id/annotation`（`{text}` 设置/空串清空 requirement）、`POST .../autopilot`（`off|ap|review` 会话级 override，`null` 清除、非法值 400）、`GET /api/models`（**脱敏**：永不返回 api_key/headers；按 provider 分组去重下拉列表）、`GET /api/skills`（仅 name/description/enabled）。
- 8 个 feature-parity 端点（`src/api_ops.rs`，于 `src/lib.rs::build_app()` 注册）：fork（`POST /api/sessions/:id/fork`，调共享实现 `opencoder_session::fork::fork_session`，404 语义在 handler 层判定）、compact、handoff、skill、config GET/PATCH（PATCH 经 `DrainCmd::ReloadConfig` 热重载）、bg list/stop。
- `/api/envs` 环境配置管理（`src/api_envs.rs`，5 路由）：`GET`（列表 + active）、`POST {name, capture_current=true}`（400 非法名 / 409 重名）、`PATCH {active: name|null}`（404 未知环境）、`POST /:name/recapture`、`DELETE /:name`（active 环境删除先清标记）。所有会改变有效配置的变更向全部 live session 扇出 `DrainCmd::ReloadConfig`（与 `PATCH /api/config` 同机制——快照 handles keys 后逐个 send_cmd）；recapture/delete 仅在影响 active 环境时扇出。`GET /api/config` 自动反映环境层（`Config::load` 解析）。
- SPA 前端 `GET /`（`src/html.rs`）：`src/assets/` 模块化为 8 个 JS 模块（api/sse/sessions/chat/composer/questions/queue_panel/settings，各 ≤400 行；render.js/app.js 已删除、职责被吸收）+ index.html/styles.css，经 `include_str!` + `LazyLock` 按依赖序在编译期拼为单一内联 HTML 文档（单二进制，无静态文件服务）。SPA 监听 18 种 SSE 事件类型（`SessionEvent` 共 21 个细粒度 kind，其余仅持久化/回放）、interrupt、steer/queue 投递、model/agent 切换、image 上传、fork/compact；question 1.5s 轮询卡片、pending 输入抽屉（删除/重排）、模型下拉来自 `/api/models`；SSE 断线重连（`/seq` + `?after=` 续订，5 次退避 1..16s，badge+banner；live 事件无 seq，重连窗口可能重复/漏少量事件，以快照+messages 兜底——已知限制）。

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
- feature-parity 端点测试：`tests/web_api_ops.rs`（fork/skill/compact/handoff/config/bg）；SPA 装配单测：`src/html.rs`。 环境管理端点：`tests/web_envs.rs`（列表/创建/激活/重捕获/删除 + ReloadConfig 扇出）。
- web/TUI 对齐端点测试（2026-08-21）：`tests/web_questions.rs`（answer 闭环/skip→SKIPPED/400/404/空列表/最后订阅者断开 abandon）、`tests/web_inputs.rs`（queue 列表/删除/重排 + 默认 steer）、`tests/web_meta_endpoints.rs`（annotation/autopilot/models 脱敏/skills 形状）、`tests/web_list_events.rs`（workdir 过滤 + Last-Event-ID replay）、`tests/web_drain_cmds.rs`（SetApMode/SetAnnotation live 应用 + ResetPlanPhase 持久化 + 切 plan 计数清零）、`tests/client_remote_ops.rs`（client 侧 18 方法 e2e）。
