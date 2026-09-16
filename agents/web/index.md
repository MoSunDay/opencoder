Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- `src/lib.rs` — `AppState` 装配
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- `src/handle.rs` — `SessionHandle` ring 缓冲 + broadcast
- `src/auth_mw.rs` — Bearer → Identity
- `src/html.rs` — SPA 产物内嵌
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`

## 接缝
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。
