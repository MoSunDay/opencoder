Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# web 模块

axum HTTP/SSE 会话管理与编译期内嵌 SPA。

## 关键路径

- `src/lib.rs` — `AppState`（store/handles/nodes/controls/project/team/brain）与路由装配。
- `src/api.rs` — `/prompt` admit 即返回；draining 中 agent 切换 409。
- `src/api_events.rs` — `/events` SSE replay+live；`/api/sessions/:id/seq` 持久化事件游标。
- `src/handle.rs` — `SessionHandle` ring 缓冲+broadcast；`admit_and_drain_guarded`/`drain_to_completion`。
- `src/handle_lifecycle.rs` — `lock_session_lifecycle` 复核同 handle，防锁旧对象。
- `src/sse_dedup.rs` — `forward_live` live 去重与 pre-subscribe gap 桥接。
- `src/auth_mw.rs` — 纯 Bearer；豁免 `/`、`/static/*`、`/api/time`、`/favicon.ico`；control 经 `#[path]` 复用。
- `src/html.rs` — SPA 产物 `include_bytes!` 内嵌，`/`+`/static/:name` 白名单。
- `src/api_ops.rs`、`src/cmd.rs` — fork/compact/handoff/skill/config/bg；`DrainCmd` 通道。
- `src/api_agents.rs`、`src/api_agent_resources.rs`、`src/api_agent_nfs.rs` — 版本化 agent 面 + NFS 导出。
- `src/api_inputs.rs`、`src/api_envs.rs` — 输入列表/删除/reorder；环境 CRUD 扇出 ReloadConfig。
- `src/api_questions.rs`、`src/handle_questions.rs` — question answer/skip 闭环。
- `src/api_subagents.rs` — 子代理任务列表；`DELETE /api/sessions?keep=` clear-all。
- `src/api_brain.rs` — brain CRUD/search/dispatch，typed 错误映射。
- `src/api_teams.rs`、`src/api_teams_topics.rs`、`src/team_state.rs`、`src/team_hub.rs` — 团队运行时与话题。
- `src/api_project*.rs` — project HTTP 适配；未 init 全部 503。
- `src/api_todo_*.rs`、`src/todo_hub.rs` — TODO 模板/环境/run 分发。
- `src/api_nodes*.rs`、`src/api_control.rs`、`src/nodes_state.rs`、`src/sse_nodes.rs` — 节点注册/心跳/claim/控制。
- `src/api_dag.rs`、`src/api_nodes_dag.rs`、`src/sse_dag.rs`、`src/dag_state.rs` — DAG CRUD/dispatch/claim/SSE。
- `spa/src/` — React18+antd SPA（vitest），产物提交于 `spa/dist`。
- `spa/src/harness/` — harness 管理、runners、启动字段。
- `spa/src/fleet/` — 节点/执行/团队/调度面板。
- `spa/src/project/` — 项目目标/里程碑/TODO 面板。
- `tests/` — HTTP/SSE 契约与节点 e2e 测试。

## 边界

- 无 LLM 单例：每 prompt 按配置构建；`client_override` 仅注入接缝。
- `opencoder-server` 走 control，不启动本模块执行面；worker 进程内复用 session router。

## 相关

- [agents/session](../session/index.md) — drain 与 cancel。
- [agents/store](../store/index.md) — 持久化与事件回放。
- [agents/control](../control/index.md) — 平台控制面。
- [agents/worker](../worker/index.md) — 节点执行面。
