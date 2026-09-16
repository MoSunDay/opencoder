Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# web 模块

axum HTTP + SSE 会话管理 + 内嵌 SPA。

## 索引
- `src/lib.rs` — `AppState` 装配
- `src/api.rs`、`src/api_*.rs` — 各域 HTTP API（prompt/events/agents/dag/todo/team/…）
- `src/api_agents.rs` — agent 目录 API：列表项含 `description`
  （soul 首行，回退 "Custom agent <name>"）与 `primary`
  （`is_primary() && name != "workflow"`，与 TUI 选择器同规则）
- `src/handle.rs` — `SessionHandle` ring 缓冲 + broadcast
- `src/auth_mw.rs` — Bearer → Identity
- `src/html.rs` — SPA 产物内嵌
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`；
  composer 命令菜单 `commandMenu.js` + `fuzzy.js`（与 TUI `/agent`、`@`
  agent 菜单同 fuzzy 语义；`@` sigil 条目只来自 agent 目录）

## 接缝
- 会话执行复用 session 运行时；持久化经 `Arc<dyn Store>`。
- `spa/src/brain/workbench/` 编辑 v2 的 input、实例、output、路由，能力选择使用注册目录；`editor/` 管理端口、映射、出口与草稿。提交具名文档后，画布和 inspector 按 visit、output、route 展示活跃分支、等待原因、完成／验证证据和局部路由读集。
- Brain 的校验、快照发布和执行由 [control](../control/index.md) 与 [brain](../brain/index.md) 提供；前端不另行推断路由或验证结果。历史旧格式只读，新写入提示迁移。
