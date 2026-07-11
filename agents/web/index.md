Commit: (working-tree, pre-initial-commit)

# web 模块

## 职责
axum HTTP/SSE 会话管理服务。提供 session CRUD、prompt 提交（admit 即返回）、事件流（SSE replay+live）、运行时 agent/model 切换、interrupt。

## 边界与非目标
- 不持有 LLM 客户端单例——每个 prompt 按配置构建 `ChatClient`。
- 非目标：鉴权 / CORS（tower-http 引入但未启用）/ 多 workdir 路由（当前单 workdir）。`serve` 默认仅绑 `127.0.0.1`（回环），需显式 `--host` 才对外。

## 关键抽象
- `AppState`（`src/lib.rs`）：`store: Arc<dyn Store>`、`workdir`、`handles: HandleMap`。
- `SessionHandle`（`src/handle.rs`）：`tx: broadcast::Sender<SseEvt>`、`cancel: Mutex<CancellationToken>`（每次 spawn 刷新，避免上次 interrupt 的永久取消毒化新 drain）、`overrides: Mutex<RuntimeOverrides>`、`draining: AtomicBool`（CAS 标记 drain 是否在跑，**与 map 存在性解耦**——否则早订阅的 `/events` 句柄会阻止 drain spawn）。
- `HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>`：活跃 drain 句柄注册表；handle 可由 `/events` 或 `/prompt` get-or-create。
- `admit_and_drain`（`src/handle.rs`）：admit 输入到 Store → get-or-create handle（共享 broadcast 通道）→ `draining.swap(true)` CAS 决定是否 spawn drain → 立即返回 admitted_seq。
- `drain_to_completion`（`src/handle.rs`）：`DrainGuard` 在 Drop（含 panic）复位 `draining`；`resume` 构建 session → 应用 overrides → `run(session, "", ...)`（drain 模式）→ on_event 同时 broadcast + 落 `session_events` 表供 SSE replay → 完成后**保留 handle 于 map**（供 late SSE replay + 后续 re-admit 再 spawn）；仅 resume 失败（session 行缺失）时移除。
- `data_dir_for`（`src/lib.rs`）：workdir → 稳定 FNV-1a 64 指纹（非 `DefaultHasher`，后者 std 不保证跨版本稳定，会让 DB 路径身份漂移）→ 本地数据目录。

## 主流程
POST /prompt（`src/api.rs`）：解析 body → load config → 建 ChatClient → `ensure_session_row` → `admit_and_drain` → 返回 `{admitted_seq}`（非阻塞）。
GET /events：`events_after(after)` 重放 + 订阅 broadcast 实时转发（BroadcastStream，lag 客户端丢帧不阻塞 runner）。
POST /agent|/model：更新 store meta + handle.overrides（下一轮 drain 生效）。
POST /interrupt：handle.cancel.cancel() → drain 在下个 turn 边界退出。

## 依赖与接口
- 依赖：axum 0.7（ws feature）、tokio-stream（sync feature，BroadcastStream）、tokio-util（CancellationToken）、opencode-session/store/llm/core。
- 被依赖：cli（serve 命令）。

## 相关模块
- [agents/session](../session/index.md) — drain 与 cancel。
- [agents/store](../store/index.md) — 持久化与事件回放。

## 代表性锚点
- 契约测试：`tests/web_contract.rs`（prompt admit 立即返回、SSE replay+live、agent 切换持久化、interrupt 取消 token、pre-existing handle 不阻塞 drain 回归）
