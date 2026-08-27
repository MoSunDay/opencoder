Commit: (working-tree, node-task status API 收尾)

# `daemon --server` 任务面 API 收尾（查询端点 + 冒烟修复 + WebUI 实机验证）

## 背景

多节点执行的**写路径**（注册/心跳/claim/上传/终态上报）已在上轮落地，但浏览器与运维侧缺**读路径**：看不到单个任务的详情、无法按状态/节点过滤全集群任务、也无法从合成 session 反查任务。本轮补齐三个查询端点与 dispatch 回执的 `status` 字段，并修复两处被 `server`/`node` 子命令 → `daemon` 化重构打破的进程级冒烟，完成回归门与 WebUI 实机验证。

## 实现

### store（crates/store）

- `list_node_tasks_filtered(node_id, status, limit)`：全集群任务列表，`?status=`/`?node_id=`/`limit` 过滤；FIFO 定序与 claim 完全一致（`created_at ASC, rowid ASC`——同毫秒 ULID 无单调性，必须 rowid tiebreak）。`limit=0` 钳到 1，上限 1000。
- `get_node_task_by_session(session_id)`：合成 session → 任务反查；普通 session 合法返回 `None`。
- 两者均为 trait default `bail!("unimplemented")` + libsql 实现，测试 fake 零破坏。

### web（crates/web/api_nodes_ops.rs）

- `GET /api/nodes/tasks?status=&node_id=&limit=` — 全局任务列表（静态路由 `/api/nodes/tasks/claim` 在 matchit 下仍优先，worker 轮询不受影响；未知 status 过滤值返回 400）。
- `GET /api/nodes/tasks/:tid` — 单任务详情 + `last_event_seq`（SSE 断线续传的 `?after=` 游标，随事件持久化增长）。
- `GET /api/sessions/:id/task` — session 反查，普通 session 404。
- `POST /api/nodes/:id/tasks` 回执新增 `status:"pending"` 字段（与 `task_id`/`session_id` 同源单事务记录）。

### 冒烟修复（scripts + tests）

- `scripts/smoke_nodes.sh` 重写：`server`/`node` 子命令 → `daemon --server` / `daemon --client`（`--workdir` 为全局参数前置）；Bearer → HMAC 签名（`x-sig-timestamp`/`x-sig`，canonical = `METHOD\npath\nts\nsha256(body)`，openssl 实现，每次请求现取新鲜 ts 天然防重放）；就绪探针改走免签名 `/api/time`；修复 `set -e` 陷阱（轮询内 curl 失败 `|| true`，不再裸死 exit 7）。新增 **checkpoint 4**：单任务详情（id/node/session/status + `last_event_seq>=1`）、`?status=`/`?node_id=` 过滤命中与未命中双向断言、session→task 反查——新端点首次获得进程级验证。
- 删除 `tests/client_server_smoke.rs`：被测的 client 面（`opencode client` 子命令族）已随 daemon 化重构移除，改动前即红；用户拍板删除。
- 修复 `tests/running_mode_switch_e2e.rs`：P3 迁走了 Bearer 但漏改 spawn 参数，残留的 `server` 子命令令二进制回退进 run 模式把 `server --host …` 当 prompt——子进程永不退出、就绪轮询的 `read()` 永久阻塞（30s deadline 断言永远到不了）。spawn 改为 `daemon --server` 后 0.06s 通过。

## WebUI 实机验证（headless 等效驱动）

真实双进程（`daemon --server --web` + `daemon --client`）走通 SPA 同款闭环：

- SPA 壳 `GET /` → 200 text/html 92KB（内嵌单二进制，无构建依赖）；免签名豁免面（`/`、`/api/time`）可达，`/api/nodes` 无签名 401 拒绝。
- 节点 idle → dispatch（回执 `status:"pending"`）→ SSE 事件流（`/api/nodes/tasks/:tid/events?after=-1`，6 帧 data，含真实 LLM `{"text":"ok"}` 回复）→ 终态 `done` 后任务详情 `last_event_seq` 增长。
- 全局列表 `?status=done`/`?node_id=` 命中、session→task 反查解析到同一任务、未知 status 过滤 400 报错——与单测契约一致。

## 回归门动作

- `cargo fmt --all`：格式化并发批次中未过 rustfmt 的文件（auth_sig.rs、web tests/support 等）。
- `crates/node/tests/runner_control.rs` 一处 `assert_eq!(…, true)` → `assert!(…)`；`crates/session/src/runner/mod.rs` 错误传播改用 `?`（同语义）——均为 clippy `-D warnings` 门。

## 边界

- checkpoint 3 沿用原语义：`error` 也算合法终态（沙箱无 LLM 配置时不构成失败）。
- `agents.md` 逻辑地图中 `opencode client`/`node` 的过时描述属 daemon 化重构作者的文档收尾，本轮不越界改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 全局列表过滤（node/status/双条件） | `filters_by_node_status_and_both` | `store/tests/node_tasks_query.rs` |
| FIFO rowid 同毫秒 tiebreak | `fifo_order_uses_rowid_tiebreak_for_same_ms` | `store/tests/node_tasks_query.rs` |
| limit 钳位边界 | `limit_caps_rows_and_zero_clamps_to_one` | `store/tests/node_tasks_query.rs` |
| session 反查命中+未命中 | `session_lookup_roundtrips_and_misses_gracefully` | `store/tests/node_tasks_query.rs` |
| 单任务详情 + SSE 游标初值 | `task_detail_returns_record_with_sse_bootstrap_seq` | `web/tests/nodes_api.rs` |
| 详情 seq 随事件/终态增长 | `task_detail_tracks_events_and_terminal_closure` | `web/tests/nodes_api.rs` |
| 全局列表过滤/排序/limit | `fleet_task_list_filters_sorts_and_limits` | `web/tests/nodes_api.rs` |
| claim 静态路由优先级不回退 | `static_claim_route_outranks_task_detail` | `web/tests/nodes_api.rs` |
| session 反查 200/404 面 | `session_reverse_lookup_resolves_and_404s_ordinary_sessions` | `web/tests/nodes_api.rs` |
| 双进程冒烟（cargo 注入调试二进制） | `smoke_script_two_process_nodes_flow_passes` | `tests/nodes_smoke_proc.rs` |
| daemon 化后的运行中模式切换 e2e | `real_server_rejects_running_mode_switches_until_idle` | `tests/running_mode_switch_e2e.rs` |
| 冒烟脚本 4 检查点（人工/CI 复核） | checkpoint 1–4 | `scripts/smoke_nodes.sh` |
