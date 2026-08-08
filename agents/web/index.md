Commit: 05d4bdf110cd7bfa75492f8ea7eebbb7cdb4c662

# web 模块

## 职责
axum HTTP/SSE 会话管理服务。提供 session CRUD、prompt 提交（admit 即返回）、事件流（SSE replay+live）、运行时 agent/model 切换、interrupt。

## 边界与非目标
- 不持有 LLM 客户端单例——每个 prompt 按配置构建 `ChatClient`。
- 非目标：CORS（tower-http 引入但未启用）/ 多 workdir 路由（当前单 workdir）。鉴权**已实现**（`auth.rs`：bearer token 中间件，token 来自 `--token` / `OPENCODER_SERVER_TOKEN` / 自动生成）。`serve` 默认仅绑 `127.0.0.1`（回环），需显式 `--host` 才对外。

## 关键抽象
- `AppState`（`src/lib.rs`）：`store: Arc<dyn Store>`、`workdir`、`handles: HandleMap`。
- `SessionHandle`（`src/handle.rs`）：`tx: broadcast::Sender<SseEvt>`、`cancel: Mutex<CancellationToken>`（每次 spawn 刷新，避免上次 interrupt 的永久取消毒化新 drain）、`overrides: Mutex<RuntimeOverrides>`、`draining: AtomicBool`（CAS 标记 drain 是否在跑，**与 map 存在性解耦**——否则早订阅的 `/events` 句柄会阻止 drain spawn），以及按 call_id 共享给 session runner 的 `child_turn_cancels` / `child_steer_gates` 注册表。
- `HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>`：活跃 drain 句柄注册表；handle 可由 `/events` 或 `/prompt` get-or-create。
- `admit_and_drain`（`src/handle.rs`）：admit 输入到 Store → get-or-create handle（共享 broadcast 通道）→ `draining.swap(true)` CAS 决定是否 spawn drain → 立即返回 admitted_seq。
- `drain_to_completion`（`src/handle.rs`）：`DrainGuard` 在 Drop（含 panic）复位 `draining`；`resume` 构建 session → 应用 overrides → `run(session, "", ...)`（drain 模式）→ on_event 同时 broadcast（`SseEvt::from_session_event` 现走 `SessionEvent::sse_kind()/sse_data()/coarse_kind()` 单一真相源）+ 落 `session_events` 表（持久化 `sse_kind`）供 SSE replay；`GET /events` 的 `get_events` replay 优先取 `sse_kind`、`None` 回退 `event_kind_str(coarse)` → 完成后**保留 handle 于 map**（供 late SSE replay + 后续 re-admit 再 spawn）；仅 resume 失败（session 行缺失）时移除。
- drain 命令通道：`SessionHandle` 另携 `cmd_tx`/`cmd_rx`（`Arc<std::sync::Mutex<CmdRx>>`，锁仅在取命令时短暂持有，绝不跨 await）。`DrainCmd` 枚举（`src/cmd.rs`）：`Compact`、`Handoff{extra}`、`SetSkill(Option<String>)`、`ReloadConfig`。需 `&mut SessionState`（仅存于 `drain_to_completion` 内）的操作经 `send_cmd()` 入队；`run()` 完成后 `process_drain_cmds()` 在 drain 闭包内排空队列。`CmdRxGuard` 在 Drop 时还原 `cmd_rx`（panic-safe）。
- `data_dir_for`（`src/lib.rs`）：workdir → 稳定 FNV-1a 64 指纹（非 `DefaultHasher`，后者 std 不保证跨版本稳定，会让 DB 路径身份漂移）→ 本地数据目录。

## 主流程
POST /prompt（`src/api.rs`）：解析 body → load config → 建 ChatClient → `ensure_session_row` → `admit_and_drain` → 返回 `{admitted_seq}`（非阻塞）。
GET /events：`events_after(after)` 重放 + 订阅 broadcast 实时转发（BroadcastStream，lag 客户端丢帧不阻塞 runner）。
GET /api/sessions/:id/seq（`get_event_seq`）：返回该 session 最高已持久化事件 seq（无则 0），供远端 client snapshot 事件游标（只取本次 prompt 产生的事件）。
POST /agent|/model：更新 store meta + handle.overrides（下一轮 drain 生效）。**RUNNING-GATE**：`draining` 为真（drain 进行中）时两者在**任何** store-meta/config/override 变更之前返回 409（`error_409`：`agent switch refused while drain running` / `model switch refused while drain running`），get-or-create handle 先行保证 override 永不被丢弃（原子性，镜像 `post_interrupt` 的 draining-gate）。`POST /model` body 新增 `persist_default: bool`（`#[serde(default)]`=false）：false（默认）= session-only（**不写盘**，与 TUI `/model` 默认一致）；true=额外 `Config::save` 写全局默认到 opencoder.json，失败返 500。
POST /interrupt：handle.cancel.cancel() → drain 在下个 turn 边界退出。
POST `/api/sessions/:id/subagents/:task_id/steer`：先校验 task 属于父 session 且为 `Running`，再从 live handle 的 child gate 取得 reservation 后写入 child session。gate 缺失/关闭返回 409 且不落 input；若写入后被强制关闭则删除该 row 后返回 409；成功提交后触发目标 child turn cancel，保持 Web 的立即打断语义。
- 8 个 feature-parity 端点（`src/api_ops.rs`，于 `src/lib.rs::build_app()` 注册）：fork（`POST /api/sessions/:id/fork`，调共享实现 `opencoder_session::fork::fork_session`，404 语义在 handler 层判定）、compact、handoff、skill、config GET/PATCH（PATCH 经 `DrainCmd::ReloadConfig` 热重载）、bg list/stop。
- SPA 前端 `GET /`（`src/html.rs`）：`src/assets/`（index.html、styles.css、render.js、app.js）经 `include_str!` + `LazyLock` 在编译期拼为单一内联 HTML 文档（单二进制，无静态文件服务）。SPA 覆盖全部 17 种 SSE 事件类型、interrupt、steer/queue 投递、model/agent 切换、image 上传、fork/compact。

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
- feature-parity 端点测试：`tests/web_api_ops.rs`（fork/skill/compact/handoff/config/bg）；SPA 装配单测：`src/html.rs`。
