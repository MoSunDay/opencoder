Commit: (working-tree, pre-initial-commit)

# web 模块

## 职责
axum HTTP/SSE 会话管理服务。提供 session CRUD、prompt 提交（admit 即返回）、事件流（SSE replay+live）、运行时 agent/model 切换、interrupt。

## 边界与非目标
- 不持有 LLM 客户端单例——每个 prompt 按配置构建 `ChatClient`。
- 非目标：鉴权 / CORS（tower-http 引入但未启用）/ 多 workdir 路由（当前单 workdir）。

## 关键抽象
- `AppState`（`src/lib.rs`）：`store: Arc<dyn Store>`、`workdir`、`handles: HandleMap`。
- `SessionHandle`（`src/handle.rs`）：`tx: broadcast::Sender<SseEvt>`、`cancel: CancellationToken`、`overrides: Mutex<RuntimeOverrides>`。
- `HandleMap = Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>`：活跃 drain 句柄注册表。
- `admit_and_drain`（`src/handle.rs`）：admit 输入到 Store → 若无活跃 drain 则 spawn 一个 → 立即返回 admitted_seq。
- `drain_to_completion`（`src/handle.rs`）：`resume` 构建 session → 应用 overrides → `run(session, "", ...)`（drain 模式）→ on_event 同时 broadcast + 落 `session_events` 表供 SSE replay → 完成后从 map 移除。

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
- 契约测试：`tests/web_contract.rs`（prompt admit 立即返回、SSE replay+live、agent 切换持久化、interrupt 取消 token）
